use crate::canonical;
use crate::cas::{CasObject, CasStore};
use crate::error::{ClewError, ErrorCode};
use crate::generation_service::{
    ensure_candidate_generation, load_candidate_generation, publish_candidate_generation,
    store_ready_generation,
};
use crate::incremental_v2::{Coverage, Support};
use crate::process_isolation::isolate_controller_authority;
use crate::python_project_model::PYTHON_GRAMMAR_AUTHORITY;
use crate::repository_snapshot::{LEGACY_EXCLUDES, capture};
use crate::session::{ContextObject, PlanObject, SessionAuthority, SessionLanguage};
use crate::state::{StateAuthority, create_private_directory};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsStr;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};

#[cfg(unix)]
use std::os::fd::AsRawFd;
#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

pub const PLAN_V2_SCHEMA: &str = "codeclew-task-plan/2.0";
pub const PREPARED_V2_SCHEMA: &str = "codeclew-prepared-candidate/5.0";
const CANDIDATE_CHECKPOINT_SCHEMA: &str = "codeclew-candidate-checkpoint/3.0";
const MAX_EDIT_BYTES: usize = 8 * 1024 * 1024;
const MAX_VALIDATION_OUTPUT_BYTES: usize = 4 * 1024 * 1024;
const MAX_PUBLIC_DIFF_BYTES: usize = 64 * 1024;
const MAX_DERIVED_OUTPUTS: usize = 16 * 1024;
const MAX_DERIVED_PATH_BYTES: usize = 1024 * 1024;
const MAX_DERIVED_CONTENT_BYTES: u64 = 8 * 1024 * 1024 * 1024;

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
    Python,
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
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ObligationSource {
    Context,
    Candidate,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct QualifiedObligation {
    pub approval_id: String,
    pub source: ObligationSource,
    pub record_digest: String,
    pub record: Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CandidateDiff {
    pub digest: String,
    pub byte_size: usize,
    pub over_limit: bool,
    pub patch: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DerivedOutput {
    pub path: String,
    pub content_digest: String,
    pub byte_size: u64,
    pub executable: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConditionalPublicationApproval {
    pub schema: String,
    pub mode: String,
    pub run_id: String,
    pub request_digest: String,
    pub session_authority_digest: String,
    pub context_id: String,
    pub context_evidence_digest: String,
    pub plan_id: String,
    pub obligations: Vec<QualifiedObligation>,
    pub candidate_commit: String,
    pub candidate_snapshot: CasObject,
    pub changed_files: Vec<String>,
    pub validation_evidence: Vec<ValidationEvidence>,
    pub prepared_authority_digest: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PreparedCandidateV2 {
    pub schema: String,
    pub session_id: String,
    pub context_id: String,
    pub context_evidence_digest: String,
    pub plan_id: String,
    pub base_revision: String,
    pub target_ref: String,
    pub target_oid: String,
    pub candidate_commit: String,
    pub candidate_snapshot: CasObject,
    pub semantic_generation: CasObject,
    pub semantic_generation_key: String,
    pub changed_files: Vec<String>,
    pub validation_evidence: Vec<ValidationEvidence>,
    pub qualified_obligations: Vec<QualifiedObligation>,
    pub conditional_publish_eligible: bool,
    pub diff: CandidateDiff,
    pub derived_outputs: Vec<DerivedOutput>,
    pub prepared_authority_digest: String,
    pub publication_blocked: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CandidateCheckpointV2 {
    schema: String,
    session_id: String,
    context_id: String,
    plan_id: String,
    base_revision: String,
    target_ref: String,
    target_oid: String,
    candidate_commit: String,
    changed_files: Vec<String>,
    validation_evidence: Vec<ValidationEvidence>,
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
                if looks_like_embedded_diff_artifact(new_text) {
                    return Err(invalid(
                        "REPLACE_TEXT newText contains embedded unified-diff prefixes",
                    ));
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

fn looks_like_embedded_diff_artifact(text: &str) -> bool {
    let mut nonempty_after_first = 0usize;
    let mut plus_prefixed = 0usize;
    for line in text.lines().skip(1).filter(|line| !line.is_empty()) {
        nonempty_after_first += 1;
        if line.starts_with('+') {
            plus_prefixed += 1;
        }
    }
    plus_prefixed >= 2 && plus_prefixed * 2 >= nonempty_after_first
}

pub fn prepare(
    session: &SessionAuthority,
    context: &ContextObject,
    plan_object: &PlanObject,
    candidate_root: &Path,
) -> Result<PreparedCandidateV2, ClewError> {
    let plan = require_mutation_request(session, context, plan_object)?;
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
    let repo = mutation_git_repository(session)?;
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
                "--no-checkout",
                worktree
                    .to_str()
                    .ok_or_else(|| invalid("candidate path is not UTF-8"))?,
                &session.base_revision,
            ])
            .current_dir(&repo),
        "candidate worktree creation failed",
    )?;
    git_status(
        Command::new("git")
            .args(["checkout", "--force", &session.base_revision, "--", "."])
            .args(LEGACY_EXCLUDES)
            .current_dir(&worktree),
        "candidate filtered checkout failed",
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
    let checkpoint = CandidateCheckpointV2 {
        schema: CANDIDATE_CHECKPOINT_SCHEMA.into(),
        session_id: session.session_id.clone(),
        context_id: context.context_id.clone(),
        plan_id: plan_object.plan_id.clone(),
        base_revision: session.base_revision.clone(),
        target_ref: session.target_ref.clone(),
        target_oid: session.target_oid.clone(),
        candidate_commit,
        changed_files,
        validation_evidence,
    };
    write_candidate_checkpoint(candidate_root, &checkpoint)?;
    finish_preparation(session, context, plan_object, candidate_root, checkpoint)
}

fn finish_preparation(
    session: &SessionAuthority,
    context: &ContextObject,
    plan_object: &PlanObject,
    candidate_root: &Path,
    checkpoint: CandidateCheckpointV2,
) -> Result<PreparedCandidateV2, ClewError> {
    validate_checkpoint(session, context, plan_object, &checkpoint)?;
    let worktree = candidate_root.join("worktree");
    if git(&worktree, &["rev-parse", "HEAD"])? != checkpoint.candidate_commit {
        return Err(ClewError::new(
            ErrorCode::WorktreeRecoveryRequired,
            "candidate worktree no longer identifies its checkpoint commit",
        ));
    }
    if git(
        &worktree,
        &["rev-parse", &format!("{}^", checkpoint.candidate_commit)],
    )? != checkpoint.base_revision
    {
        return Err(ClewError::new(
            ErrorCode::WorktreeRecoveryRequired,
            "candidate checkpoint is not a direct child of the session base",
        ));
    }
    let plan = validate_plan_value(&plan_object.plan)?;
    let expected_files = planned_files(&plan);
    let committed_files = nul_paths(
        &worktree,
        &[
            "diff",
            "--name-only",
            "-z",
            &checkpoint.base_revision,
            &checkpoint.candidate_commit,
            "--",
        ],
    )?;
    if expected_files != checkpoint.changed_files.iter().cloned().collect()
        || committed_files != expected_files
    {
        return Err(ClewError::new(
            ErrorCode::WorktreeRecoveryRequired,
            "candidate checkpoint write set differs from its plan or commit",
        ));
    }
    require_clean(&worktree)?;
    let state = StateAuthority::process_default()?;
    let store = CasStore::open(&state)?;
    let semantic = ensure_candidate_generation(
        session,
        &worktree,
        &checkpoint.candidate_commit,
        &candidate_root.join("staged-generation.json"),
    )?;
    let semantic_generation = store_ready_generation(&store, &semantic)?;
    let (qualified_obligations, context_strict, context_conditional) =
        context_publication_authority(context)?;
    let mut qualified_obligations = qualified_obligations;
    qualified_obligations.extend(candidate_obligations(&semantic)?);
    let qualified_obligations = normalize_qualified_obligations(qualified_obligations)?;
    let candidate_strict = semantic.completeness.publishable();
    let candidate_conditional = semantic.completeness.support == Support::Supported
        && matches!(
            semantic.completeness.coverage,
            Coverage::Complete { .. } | Coverage::Partial { .. }
        )
        && semantic.certainty == "UNSURE"
        && !semantic.completeness.obligations.is_empty();
    let publication_blocked = !(context_strict && candidate_strict);
    let conditional_publish_eligible = publication_blocked
        && (context_conditional || context_strict)
        && (candidate_conditional || candidate_strict)
        && (context_conditional || candidate_conditional)
        && !qualified_obligations.is_empty()
        && checkpoint
            .validation_evidence
            .iter()
            .all(|item| item.success);
    require_clean(&worktree)?;
    let (_, candidate_snapshot) = capture(&worktree, &store)?;
    let diff = candidate_diff(
        &worktree,
        &checkpoint.base_revision,
        &checkpoint.candidate_commit,
        &checkpoint.changed_files,
    )?;
    let derived_outputs = capture_derived_outputs(&worktree)?;
    let mut prepared = PreparedCandidateV2 {
        schema: PREPARED_V2_SCHEMA.into(),
        session_id: session.session_id.clone(),
        context_id: context.context_id.clone(),
        context_evidence_digest: context.evidence_digest.clone(),
        plan_id: plan_object.plan_id.clone(),
        base_revision: session.base_revision.clone(),
        target_ref: session.target_ref.clone(),
        target_oid: session.target_oid.clone(),
        candidate_commit: checkpoint.candidate_commit,
        candidate_snapshot,
        semantic_generation,
        semantic_generation_key: semantic.generation_key,
        changed_files: checkpoint.changed_files,
        validation_evidence: checkpoint.validation_evidence,
        qualified_obligations,
        conditional_publish_eligible,
        diff,
        derived_outputs,
        prepared_authority_digest: String::new(),
        publication_blocked,
    };
    prepared.prepared_authority_digest = prepared_authority_digest(&prepared)?;
    Ok(prepared)
}

pub fn recover_preparation(
    session: &SessionAuthority,
    context: &ContextObject,
    plan_object: &PlanObject,
    candidate_root: &Path,
) -> Result<PreparedCandidateV2, ClewError> {
    let plan = require_mutation_request(session, context, plan_object)?;
    let mut checkpoint = if candidate_root.join("checkpoint-v2.json").exists() {
        load_candidate_checkpoint(candidate_root)?
    } else {
        reconstruct_candidate_checkpoint(session, context, plan_object, candidate_root)?
    };
    validate_checkpoint(session, context, plan_object, &checkpoint)?;
    checkpoint.validation_evidence =
        run_validation(&candidate_root.join("worktree"), &plan.validation)?;
    finish_preparation(session, context, plan_object, candidate_root, checkpoint)
}

fn reconstruct_candidate_checkpoint(
    session: &SessionAuthority,
    context: &ContextObject,
    plan_object: &PlanObject,
    candidate_root: &Path,
) -> Result<CandidateCheckpointV2, ClewError> {
    let worktree = candidate_root.join("worktree");
    let candidate_commit =
        recoverable_candidate_commit(session, candidate_root)?.ok_or_else(|| {
            ClewError::new(
                ErrorCode::WorktreeRecoveryRequired,
                "candidate has no committed OID to recover",
            )
        })?;
    require_clean(&worktree)?;
    let plan = validate_plan_value(&plan_object.plan)?;
    let checkpoint = CandidateCheckpointV2 {
        schema: CANDIDATE_CHECKPOINT_SCHEMA.into(),
        session_id: session.session_id.clone(),
        context_id: context.context_id.clone(),
        plan_id: plan_object.plan_id.clone(),
        base_revision: session.base_revision.clone(),
        target_ref: session.target_ref.clone(),
        target_oid: session.target_oid.clone(),
        candidate_commit,
        changed_files: planned_files(&plan).into_iter().collect(),
        validation_evidence: run_validation(&worktree, &plan.validation)?,
    };
    validate_checkpoint(session, context, plan_object, &checkpoint)?;
    write_candidate_checkpoint(candidate_root, &checkpoint)?;
    Ok(checkpoint)
}

pub fn checkpoint_commit(candidate_root: &Path) -> Result<Option<String>, ClewError> {
    let path = candidate_root.join("checkpoint-v2.json");
    if !path.exists() {
        return Ok(None);
    }
    Ok(Some(
        load_candidate_checkpoint(candidate_root)?.candidate_commit,
    ))
}

pub fn recoverable_candidate_commit(
    session: &SessionAuthority,
    candidate_root: &Path,
) -> Result<Option<String>, ClewError> {
    let worktree = candidate_root.join("worktree");
    if !worktree.exists() {
        return Ok(None);
    }
    let head = git(&worktree, &["rev-parse", "HEAD"])?;
    if head == session.base_revision {
        return Ok(None);
    }
    if !git_oid(&head)
        || git(&worktree, &["rev-parse", &format!("{head}^")])? != session.base_revision
    {
        return Err(ClewError::new(
            ErrorCode::WorktreeRecoveryRequired,
            "candidate worktree has an unexpected committed history",
        ));
    }
    Ok(Some(head))
}

pub fn discard_precommit_candidate(
    session: &SessionAuthority,
    candidate_root: &Path,
) -> Result<bool, ClewError> {
    let worktree = candidate_root.join("worktree");
    if !worktree.exists() {
        return Ok(false);
    }
    if checkpoint_commit(candidate_root)?.is_some()
        || recoverable_candidate_commit(session, candidate_root)?.is_some()
    {
        return Err(ClewError::new(
            ErrorCode::WorktreeRecoveryRequired,
            "committed candidate cannot be discarded automatically",
        ));
    }
    git_status(
        Command::new("git")
            .args(["worktree", "remove", "--force"])
            .arg(&worktree)
            .current_dir(mutation_git_repository(session)?),
        "derived pre-commit candidate cleanup failed",
    )?;
    Ok(true)
}

fn write_candidate_checkpoint(
    candidate_root: &Path,
    checkpoint: &CandidateCheckpointV2,
) -> Result<(), ClewError> {
    let state = StateAuthority::process_default()?;
    state.write_private_atomic(
        &candidate_root.join("checkpoint-v2.json"),
        &canonical::bytes(checkpoint).map_err(internal)?,
    )
}

fn load_candidate_checkpoint(candidate_root: &Path) -> Result<CandidateCheckpointV2, ClewError> {
    let path = candidate_root.join("checkpoint-v2.json");
    let metadata = fs::symlink_metadata(&path).map_err(io_error)?;
    if metadata.file_type().is_symlink() || !metadata.is_file() || metadata.len() > 4 * 1024 * 1024
    {
        return Err(invalid(
            "candidate checkpoint is missing, unsafe, or oversized",
        ));
    }
    let bytes = fs::read(path).map_err(io_error)?;
    let checkpoint: CandidateCheckpointV2 = serde_json::from_slice(&bytes).map_err(parse_error)?;
    if canonical::bytes(&checkpoint).map_err(internal)? != bytes {
        return Err(invalid("candidate checkpoint is not canonical"));
    }
    Ok(checkpoint)
}

fn validate_checkpoint(
    session: &SessionAuthority,
    context: &ContextObject,
    plan: &PlanObject,
    checkpoint: &CandidateCheckpointV2,
) -> Result<(), ClewError> {
    if checkpoint.schema != CANDIDATE_CHECKPOINT_SCHEMA
        || checkpoint.session_id != session.session_id
        || checkpoint.context_id != context.context_id
        || checkpoint.plan_id != plan.plan_id
        || checkpoint.base_revision != session.base_revision
        || checkpoint.target_ref != session.target_ref
        || checkpoint.target_oid != session.target_oid
        || !git_oid(&checkpoint.candidate_commit)
        || checkpoint.changed_files.is_empty()
        || checkpoint.validation_evidence.is_empty()
        || checkpoint
            .validation_evidence
            .iter()
            .any(|evidence| !evidence.success)
        || !checkpoint
            .changed_files
            .windows(2)
            .all(|pair| pair[0] < pair[1])
    {
        return Err(ClewError::new(
            ErrorCode::WorktreeRecoveryRequired,
            "candidate checkpoint differs from immutable run authority",
        ));
    }
    Ok(())
}

fn planned_files(plan: &TaskPlanV2) -> BTreeSet<String> {
    plan.operations
        .iter()
        .map(|operation| match operation {
            FileOperation::ReplaceText { target, .. }
            | FileOperation::DeleteFile { target, .. } => target.file_id.clone(),
            FileOperation::CreateFile { target, .. } => target.file_id.clone(),
        })
        .collect()
}

fn context_publication_authority(
    context: &ContextObject,
) -> Result<(Vec<QualifiedObligation>, bool, bool), ClewError> {
    let evidence = context
        .evidence
        .get("context")
        .ok_or_else(|| invalid("context publication evidence is missing"))?;
    let completeness = evidence
        .get("completeness")
        .ok_or_else(|| invalid("context completeness is missing"))?;
    let status = completeness
        .get("status")
        .and_then(Value::as_str)
        .ok_or_else(|| invalid("context completeness status is missing"))?;
    let support = completeness.get("support").and_then(Value::as_str);
    let coverage = completeness.get("coverage").and_then(Value::as_str);
    let certainty = completeness.get("certainty").and_then(Value::as_str);
    let obligations = evidence
        .get("verificationObligations")
        .and_then(Value::as_array)
        .ok_or_else(|| invalid("context obligation set is missing"))?;
    let strict = status == "COMPLETE_TASK"
        && support == Some("SUPPORTED")
        && certainty == Some("VERIFIED")
        && obligations.is_empty();
    let conditional = status == "CONDITIONAL_TASK"
        && support == Some("SUPPORTED")
        && matches!(coverage, Some("QUERY_COMPLETE" | "PARTIAL"))
        && certainty == Some("UNSURE")
        && evidence
            .get("matches")
            .and_then(Value::as_array)
            .is_some_and(|facts| !facts.is_empty())
        && !obligations.is_empty();
    let qualified = obligations
        .iter()
        .map(|record| qualify_obligation(ObligationSource::Context, record.clone()))
        .collect::<Result<Vec<_>, _>>()?;
    Ok((qualified, strict, conditional))
}

fn candidate_obligations(
    semantic: &crate::generation_service::ReadyGenerationSet,
) -> Result<Vec<QualifiedObligation>, ClewError> {
    semantic
        .completeness
        .obligations
        .iter()
        .map(|obligation| {
            qualify_obligation(
                ObligationSource::Candidate,
                serde_json::to_value(obligation).map_err(internal)?,
            )
        })
        .collect()
}

fn qualify_obligation(
    source: ObligationSource,
    record: Value,
) -> Result<QualifiedObligation, ClewError> {
    if !record.is_object() {
        return Err(invalid("conditional obligation record is not an object"));
    }
    let record_digest = canonical::hash(&record).map_err(internal)?;
    let prefix = match source {
        ObligationSource::Context => "context",
        ObligationSource::Candidate => "candidate",
    };
    Ok(QualifiedObligation {
        approval_id: format!("{prefix}:{record_digest}"),
        source,
        record_digest,
        record,
    })
}

fn normalize_qualified_obligations(
    mut obligations: Vec<QualifiedObligation>,
) -> Result<Vec<QualifiedObligation>, ClewError> {
    obligations.sort_by(|left, right| left.approval_id.cmp(&right.approval_id));
    let mut normalized = Vec::<QualifiedObligation>::with_capacity(obligations.len());
    for obligation in obligations {
        if let Some(previous) = normalized.last()
            && previous.approval_id == obligation.approval_id
        {
            if previous != &obligation {
                return Err(invalid(
                    "conditional obligation authority has a conflicting identity",
                ));
            }
            continue;
        }
        normalized.push(obligation);
    }
    Ok(normalized)
}

fn candidate_diff(
    worktree: &Path,
    base_revision: &str,
    candidate_commit: &str,
    changed_files: &[String],
) -> Result<CandidateDiff, ClewError> {
    let mut child = Command::new("git")
        .args([
            "-c",
            "core.quotePath=false",
            "diff",
            "--no-ext-diff",
            "--no-color",
            "--unified=3",
            base_revision,
            candidate_commit,
            "--",
        ])
        .args(changed_files)
        .current_dir(worktree)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(io_error)?;
    let mut stdout = child
        .stdout
        .take()
        .ok_or_else(|| internal("candidate diff pipe is unavailable"))?;
    let mut hasher = Sha256::new();
    let mut prefix = Vec::with_capacity(MAX_PUBLIC_DIFF_BYTES.saturating_add(1));
    let mut byte_size = 0usize;
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = stdout.read(&mut buffer).map_err(io_error)?;
        if read == 0 {
            break;
        }
        byte_size = byte_size
            .checked_add(read)
            .ok_or_else(|| resource("candidate diff size overflow"))?;
        hasher.update(&buffer[..read]);
        if prefix.len() <= MAX_PUBLIC_DIFF_BYTES {
            let remaining = MAX_PUBLIC_DIFF_BYTES
                .saturating_add(1)
                .saturating_sub(prefix.len());
            prefix.extend_from_slice(&buffer[..read.min(remaining)]);
        }
    }
    if !child.wait().map_err(io_error)?.success() {
        return Err(invalid("candidate diff is unavailable"));
    }
    let digest = format!("sha256:{}", hex::encode(hasher.finalize()));
    let over_limit = byte_size > MAX_PUBLIC_DIFF_BYTES;
    let patch = if over_limit {
        None
    } else {
        Some(String::from_utf8(prefix).map_err(|_| invalid("candidate diff is not valid UTF-8"))?)
    };
    Ok(CandidateDiff {
        digest,
        byte_size,
        over_limit,
        patch,
    })
}

fn capture_derived_outputs(worktree: &Path) -> Result<Vec<DerivedOutput>, ClewError> {
    let mut paths = git_other_paths(
        worktree,
        &["ls-files", "--others", "--exclude-standard", "-z"],
    )?;
    paths.extend(git_other_paths(
        worktree,
        &[
            "ls-files",
            "--others",
            "--ignored",
            "--exclude-standard",
            "-z",
        ],
    )?);
    let path_bytes = paths.iter().try_fold(0usize, |total, path| {
        total
            .checked_add(path.len())
            .ok_or_else(|| resource("candidate derived-output path size overflow"))
    })?;
    if paths.len() > MAX_DERIVED_OUTPUTS || path_bytes > MAX_DERIVED_PATH_BYTES {
        return Err(resource(
            "candidate derived-output inventory exceeds count or path limits",
        ));
    }
    let mut outputs = Vec::with_capacity(paths.len());
    let mut content_bytes = 0u64;
    for path in paths {
        let absolute = worktree.join(&path);
        let metadata = fs::symlink_metadata(&absolute).map_err(io_error)?;
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err(invalid("candidate derived output is not a regular file"));
        }
        content_bytes = content_bytes
            .checked_add(metadata.len())
            .ok_or_else(|| resource("candidate derived-output size overflow"))?;
        if content_bytes > MAX_DERIVED_CONTENT_BYTES {
            return Err(resource(
                "candidate derived-output content exceeds the hashing limit",
            ));
        }
        let mut file = File::open(&absolute).map_err(io_error)?;
        let mut hasher = Sha256::new();
        let mut buffer = [0u8; 64 * 1024];
        loop {
            let read = file.read(&mut buffer).map_err(io_error)?;
            if read == 0 {
                break;
            }
            hasher.update(&buffer[..read]);
        }
        outputs.push(DerivedOutput {
            path,
            content_digest: format!("sha256:{}", hex::encode(hasher.finalize())),
            byte_size: metadata.len(),
            executable: derived_output_executable(&metadata),
        });
    }
    Ok(outputs)
}

fn git_other_paths(worktree: &Path, args: &[&str]) -> Result<BTreeSet<String>, ClewError> {
    let output = Command::new("git")
        .args(args)
        .current_dir(worktree)
        .stdin(Stdio::null())
        .output()
        .map_err(io_error)?;
    if !output.status.success() {
        return Err(invalid("candidate derived-output inventory is unavailable"));
    }
    output
        .stdout
        .split(|byte| *byte == 0)
        .filter(|row| !row.is_empty())
        .map(|row| {
            let path = std::str::from_utf8(row)
                .map_err(|_| invalid("candidate derived-output path is not UTF-8"))?;
            if !safe_path(path) || path == ".git" || path.starts_with(".git/") {
                return Err(invalid("candidate derived-output path is unsafe"));
            }
            Ok(path.to_owned())
        })
        .collect()
}

#[cfg(unix)]
fn derived_output_executable(metadata: &fs::Metadata) -> bool {
    metadata.permissions().mode() & 0o111 != 0
}

#[cfg(not(unix))]
fn derived_output_executable(_metadata: &fs::Metadata) -> bool {
    false
}

pub fn remove_exact_derived_outputs(
    worktree: &Path,
    expected: &[DerivedOutput],
) -> Result<(), ClewError> {
    verify_exact_derived_outputs(worktree, expected)?;
    let mut parents = BTreeSet::new();
    for output in expected.iter().rev() {
        let path = worktree.join(&output.path);
        fs::remove_file(&path).map_err(io_error)?;
        let mut parent = path.parent();
        while let Some(directory) = parent {
            if directory == worktree {
                break;
            }
            parents.insert(directory.to_path_buf());
            parent = directory.parent();
        }
    }
    let mut parents = parents.into_iter().collect::<Vec<_>>();
    parents.sort_by_key(|path| std::cmp::Reverse(path.components().count()));
    for directory in parents {
        match fs::remove_dir(&directory) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::DirectoryNotEmpty => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(io_error(error)),
        }
    }
    if !capture_derived_outputs(worktree)?.is_empty() {
        return Err(ClewError::new(
            ErrorCode::PreconditionFailed,
            "candidate derived-output cleanup is incomplete",
        ));
    }
    Ok(())
}

pub fn verify_exact_derived_outputs(
    worktree: &Path,
    expected: &[DerivedOutput],
) -> Result<(), ClewError> {
    if capture_derived_outputs(worktree)? != expected {
        return Err(ClewError::new(
            ErrorCode::PreconditionFailed,
            "candidate derived outputs differ from prepared authority",
        ));
    }
    Ok(())
}

pub fn verify_prepared_for_gc(
    session: &SessionAuthority,
    prepared: &PreparedCandidateV2,
    worktree: &Path,
) -> Result<(), ClewError> {
    validate_prepared(session, prepared)?;
    if git(worktree, &["rev-parse", "HEAD"])? != prepared.candidate_commit {
        return Err(ClewError::new(
            ErrorCode::PreconditionFailed,
            "published candidate HEAD differs from prepared authority",
        ));
    }
    verify_candidate_snapshot(worktree, &prepared.candidate_snapshot)?;
    verify_exact_derived_outputs(worktree, &prepared.derived_outputs)
}

fn prepared_authority_digest(prepared: &PreparedCandidateV2) -> Result<String, ClewError> {
    let mut unsigned = prepared.clone();
    unsigned.prepared_authority_digest.clear();
    canonical::hash(&unsigned).map_err(internal)
}

pub fn public_candidate_status(prepared: &PreparedCandidateV2) -> Result<Value, ClewError> {
    if prepared.prepared_authority_digest != prepared_authority_digest(prepared)? {
        return Err(ClewError::new(
            ErrorCode::PreconditionFailed,
            "prepared candidate digest is invalid",
        ));
    }
    let obligations = prepared
        .qualified_obligations
        .iter()
        .map(|item| {
            json!({
                "approvalId":item.approval_id,
                "source":item.source,
                "recordDigest":item.record_digest,
                "record":item.record,
            })
        })
        .collect::<Vec<_>>();
    let mut value = json!({
        "schema":"codeclew-public-candidate-status/1.0",
        "candidateCommit":prepared.candidate_commit,
        "changedFiles":prepared.changed_files,
        "conditionalPublishEligible":prepared.conditional_publish_eligible,
        "certainty":if prepared.publication_blocked { "UNSURE" } else { "VERIFIED" },
        "diff":prepared.diff,
        "preparedAuthorityDigest":prepared.prepared_authority_digest,
        "qualifiedObligations":obligations,
        "validationEvidence":prepared.validation_evidence,
        "derivedOutputs":{
            "count":prepared.derived_outputs.len(),
            "treeDigest":canonical::hash(&prepared.derived_outputs).map_err(internal)?,
        },
    });
    if canonical::bytes(&value).map_err(internal)?.len() > 64 * 1024 {
        value["qualifiedObligations"] = Value::Array(
            prepared
                .qualified_obligations
                .iter()
                .map(|item| {
                    json!({
                        "approvalId":item.approval_id,
                        "source":item.source,
                        "recordDigest":item.record_digest,
                    })
                })
                .collect(),
        );
        value["obligationRecordsOmitted"] = Value::Bool(true);
    }
    if canonical::bytes(&value).map_err(internal)?.len() > 64 * 1024 {
        return Err(resource("public candidate status exceeds 64 KiB"));
    }
    Ok(value)
}

pub fn conditional_approval(
    session: &SessionAuthority,
    prepared: &PreparedCandidateV2,
    run_id: &str,
    request_digest: &str,
    acknowledged: &[String],
) -> Result<ConditionalPublicationApproval, ClewError> {
    validate_prepared(session, prepared)?;
    if !prepared.publication_blocked || !prepared.conditional_publish_eligible {
        return Err(ClewError::new(
            ErrorCode::IncompleteSemanticAnalysis,
            "candidate is not eligible for conditional publication",
        ));
    }
    let mut observed = acknowledged.to_vec();
    observed.sort();
    if observed.is_empty() || observed.windows(2).any(|pair| pair[0] == pair[1]) {
        return Err(invalid(
            "conditional acknowledgement set is empty or duplicated",
        ));
    }
    let expected = prepared
        .qualified_obligations
        .iter()
        .map(|item| item.approval_id.clone())
        .collect::<Vec<_>>();
    if observed != expected {
        return Err(ClewError::new(
            ErrorCode::PreconditionFailed,
            "conditional acknowledgement set differs from prepared obligations",
        ));
    }
    if prepared.validation_evidence.is_empty()
        || prepared
            .validation_evidence
            .iter()
            .any(|item| !item.success)
    {
        return Err(ClewError::new(
            ErrorCode::TestFailed,
            "conditional publication requires successful validation evidence",
        ));
    }
    Ok(ConditionalPublicationApproval {
        schema: "codeclew-conditional-publication-approval/1.0".into(),
        mode: "ACKNOWLEDGED_UNSURE".into(),
        run_id: run_id.into(),
        request_digest: request_digest.into(),
        session_authority_digest: session.authority_digest.clone(),
        context_id: prepared.context_id.clone(),
        context_evidence_digest: prepared.context_evidence_digest.clone(),
        plan_id: prepared.plan_id.clone(),
        obligations: prepared.qualified_obligations.clone(),
        candidate_commit: prepared.candidate_commit.clone(),
        candidate_snapshot: prepared.candidate_snapshot.clone(),
        changed_files: prepared.changed_files.clone(),
        validation_evidence: prepared.validation_evidence.clone(),
        prepared_authority_digest: prepared.prepared_authority_digest.clone(),
    })
}

pub fn publish(
    session: &SessionAuthority,
    prepared: &PreparedCandidateV2,
    candidate_root: &Path,
    approval: Option<&ConditionalPublicationApproval>,
) -> Result<Value, ClewError> {
    validate_prepared(session, prepared)?;
    if prepared.publication_blocked && approval.is_none() {
        return Err(ClewError::new(
            ErrorCode::IncompleteSemanticAnalysis,
            "conditional candidate requires explicit publication approval",
        ));
    }
    if !prepared.publication_blocked && approval.is_some() {
        return Err(invalid(
            "strict candidate does not accept conditional publication approval",
        ));
    }
    let repo = session.target_repository_path()?;
    let state = StateAuthority::process_default()?;
    let repository = state.repository(&repo)?;
    let _publish_lock = RepositoryPublishLock::acquire(&state, &repository.key)?;
    let worktree = candidate_root.join("worktree");
    let source = if session.language == SessionLanguage::Python {
        None
    } else {
        Some(session.repository_path()?)
    };
    let inventory =
        require_publish_worktrees(session, prepared, &repo, source.as_deref(), &worktree)?;
    verify_candidate_snapshot(&worktree, &prepared.candidate_snapshot)?;
    let store = CasStore::open(&state)?;
    let semantic = load_candidate_generation(
        &store,
        &prepared.semantic_generation,
        session,
        &prepared.candidate_commit,
        true,
    )?;
    if semantic.generation_key != prepared.semantic_generation_key {
        return Err(ClewError::new(
            ErrorCode::IncompleteSemanticAnalysis,
            "candidate semantic generation differs from prepared authority",
        ));
    }
    let conditional = if let Some(approval) = approval {
        validate_conditional_approval(session, prepared, approval)?;
        true
    } else {
        if !semantic.completeness.publishable() {
            return Err(ClewError::new(
                ErrorCode::IncompleteSemanticAnalysis,
                "candidate semantic generation is not publication-ready",
            ));
        }
        false
    };
    let current = git(&repo, &["rev-parse", &session.target_ref])?;
    if current == prepared.candidate_commit {
        synchronize_checked_out_target(&repo, prepared)?;
        publish_candidate_generation(session, &semantic).map_err(publication_recovery_error)?;
        verify_published_worktrees(session, prepared, &repo, &inventory)?;
        return Ok(json!({
            "schema":"codeclew-publish-result/2.0",
            "status":if conditional { "PUBLISHED_CONDITIONAL" } else { "PUBLISHED" },
            "candidateCommit":prepared.candidate_commit,
            "recovered":true,
            "certainty":if conditional { "UNSURE" } else { "VERIFIED" },
            "conditionalApproval":approval,
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
    publish_candidate_generation(session, &semantic).map_err(publication_recovery_error)?;
    verify_published_worktrees(session, prepared, &repo, &inventory)?;
    Ok(json!({
        "schema":"codeclew-publish-result/2.0",
        "status":if conditional { "PUBLISHED_CONDITIONAL" } else { "PUBLISHED" },
        "candidateCommit":prepared.candidate_commit,
        "recovered":false,
        "certainty":if conditional { "UNSURE" } else { "VERIFIED" },
        "conditionalApproval":approval,
    }))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MutationProfile {
    Kotlin24Gradle,
    RustSyntax,
    PythonSyntax,
}

pub fn require_mutation_request(
    session: &SessionAuthority,
    context: &ContextObject,
    plan_object: &PlanObject,
) -> Result<TaskPlanV2, ClewError> {
    let plan = validate_plan_value(&plan_object.plan)?;
    let profile = qualified_mutation_profile(session, context)?;
    require_profile_validation(profile, &plan)?;
    Ok(plan)
}

fn qualified_mutation_profile(
    session: &SessionAuthority,
    context: &ContextObject,
) -> Result<MutationProfile, ClewError> {
    let evidence = context
        .evidence
        .get("context")
        .ok_or_else(|| unsupported_profile("context evidence is missing"))?;
    if evidence.get("language").and_then(Value::as_str) != Some(session.language.uri()) {
        return Err(unsupported_profile(
            "context language differs from the session mutation authority",
        ));
    }
    let versions = evidence
        .get("compilerVersions")
        .and_then(Value::as_object)
        .ok_or_else(|| unsupported_profile("context compiler authority is missing"))?;
    if versions.len() != session.compilations.len()
        || session.compilations.iter().any(|compilation| {
            !versions
                .get(compilation)
                .is_some_and(|version| version.is_string())
        })
    {
        return Err(unsupported_profile(
            "context compilation authority differs from the session",
        ));
    }
    let has_gradle_wrapper = if session.language == SessionLanguage::Kotlin {
        let repository = session.target_repository_path()?;
        let wrapper = repository.join("gradlew");
        fs::symlink_metadata(wrapper)
            .is_ok_and(|metadata| !metadata.file_type().is_symlink() && metadata.is_file())
    } else {
        false
    };
    mutation_profile_for(
        session.language,
        &session.compilations,
        versions,
        has_gradle_wrapper,
    )
}

fn mutation_profile_for(
    language: SessionLanguage,
    compilations: &[String],
    versions: &serde_json::Map<String, Value>,
    has_gradle_wrapper: bool,
) -> Result<MutationProfile, ClewError> {
    match language {
        SessionLanguage::Kotlin => {
            if compilations.len() != 1
                || versions
                    .values()
                    .any(|version| version.as_str() != Some("2.4.10"))
                || !has_gradle_wrapper
            {
                return Err(unsupported_profile(
                    "Kotlin mutation requires the qualified 2.4.10 single-compilation Gradle profile",
                ));
            }
            Ok(MutationProfile::Kotlin24Gradle)
        }
        SessionLanguage::Rust => {
            if versions.values().any(|version| {
                !version
                    .as_str()
                    .and_then(|value| value.lines().next())
                    .is_some_and(|value| value.starts_with("rustc 1.92.0 "))
            }) {
                return Err(unsupported_profile(
                    "Rust mutation requires the pinned Rust 1.92 syntax profile",
                ));
            }
            Ok(MutationProfile::RustSyntax)
        }
        SessionLanguage::Python => {
            if versions
                .values()
                .any(|version| version.as_str() != Some(PYTHON_GRAMMAR_AUTHORITY))
            {
                return Err(unsupported_profile(
                    "Python mutation requires the qualified tree-sitter syntax profile",
                ));
            }
            Ok(MutationProfile::PythonSyntax)
        }
    }
}

fn require_profile_validation(
    profile: MutationProfile,
    plan: &TaskPlanV2,
) -> Result<(), ClewError> {
    let matches = match profile {
        MutationProfile::Kotlin24Gradle => plan
            .validation
            .iter()
            .all(|step| step.launcher == ValidationLauncher::Gradle),
        MutationProfile::RustSyntax => plan
            .validation
            .iter()
            .all(|step| step.launcher == ValidationLauncher::Cargo),
        MutationProfile::PythonSyntax => plan.validation.iter().all(|step| {
            step.launcher == ValidationLauncher::Python
                && step.args.len() >= 2
                && step.args[0] == "-m"
                && safe_python_module(&step.args[1])
        }),
    };
    if !matches {
        return Err(unsupported_profile(
            "task validation launchers differ from the qualified mutation profile",
        ));
    }
    Ok(())
}

fn safe_python_module(module: &str) -> bool {
    !module.is_empty()
        && module.len() <= 255
        && module.split('.').all(|segment| {
            let mut characters = segment.chars();
            characters
                .next()
                .is_some_and(|character| character == '_' || character.is_ascii_alphabetic())
                && characters.all(|character| character == '_' || character.is_ascii_alphanumeric())
        })
}

fn unsupported_profile(message: &str) -> ClewError {
    ClewError::new(ErrorCode::UnsupportedProjectConfiguration, message)
}

fn publication_recovery_error(error: ClewError) -> ClewError {
    ClewError {
        code: ErrorCode::WorktreeRecoveryRequired,
        message: format!(
            "candidate ref is published but staged semantic index publication failed: {}",
            error.message
        ),
        retryable: true,
        ..error
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct WorktreeView {
    path: PathBuf,
    head: String,
    branch: Option<String>,
}

struct RepositoryPublishLock(File);

impl RepositoryPublishLock {
    fn acquire(state: &StateAuthority, repository_key: &str) -> Result<Self, ClewError> {
        if repository_key.len() != 64
            || !repository_key
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        {
            return Err(invalid("repository publish lock key is invalid"));
        }
        let path = state
            .locks_root()
            .join(format!("publish-{repository_key}.lock"));
        let mut options = OpenOptions::new();
        options.create(true).read(true).write(true);
        #[cfg(unix)]
        options.mode(0o600);
        let file = options.open(path).map_err(io_error)?;
        #[cfg(unix)]
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) } != 0 {
            return Err(io_error(std::io::Error::last_os_error()));
        }
        Ok(Self(file))
    }
}

impl Drop for RepositoryPublishLock {
    fn drop(&mut self) {
        #[cfg(unix)]
        unsafe {
            libc::flock(self.0.as_raw_fd(), libc::LOCK_UN);
        }
    }
}

fn require_publish_worktrees(
    session: &SessionAuthority,
    prepared: &PreparedCandidateV2,
    target: &Path,
    source: Option<&Path>,
    candidate: &Path,
) -> Result<Vec<WorktreeView>, ClewError> {
    let inventory = worktree_inventory(target)?;
    let target = target.canonicalize().map_err(io_error)?;
    let source = source
        .map(|source| source.canonicalize().map_err(io_error))
        .transpose()?;
    let candidate = candidate.canonicalize().map_err(io_error)?;
    let mut target_branch_count = 0usize;
    let mut source_found = source.is_none();
    let mut candidate_found = false;
    for item in &inventory {
        let path = item.path.canonicalize().map_err(io_error)?;
        if item.branch.as_deref() == Some(session.target_ref.as_str()) {
            target_branch_count += 1;
            if path != target
                || (item.head != prepared.target_oid && item.head != prepared.candidate_commit)
            {
                return Err(ClewError::new(
                    ErrorCode::PreconditionFailed,
                    "session target ref is checked out by an unexpected worktree",
                ));
            }
            require_clean(&path)?;
        }
        if source.as_ref().is_some_and(|source| path == *source) {
            source_found = item.branch.is_none() && item.head == session.base_revision;
        }
        if path == candidate {
            candidate_found = item.branch.is_none() && item.head == prepared.candidate_commit;
        }
    }
    if target_branch_count > 1 || !source_found || !candidate_found {
        return Err(ClewError::new(
            ErrorCode::WorktreeRecoveryRequired,
            "session worktree inventory differs from prepared publication authority",
        ));
    }
    Ok(inventory)
}

fn mutation_git_repository(session: &SessionAuthority) -> Result<PathBuf, ClewError> {
    if session.language == SessionLanguage::Python {
        session.target_repository_path()
    } else {
        session.repository_path()
    }
}

fn verify_published_worktrees(
    session: &SessionAuthority,
    prepared: &PreparedCandidateV2,
    target: &Path,
    before: &[WorktreeView],
) -> Result<(), ClewError> {
    let after = worktree_inventory(target)?;
    if after.len() != before.len()
        || after.iter().zip(before).any(|(observed, prior)| {
            observed.path != prior.path
                || observed.branch != prior.branch
                || (observed.branch.as_deref() != Some(session.target_ref.as_str())
                    && observed.head != prior.head)
        })
    {
        return Err(ClewError::new(
            ErrorCode::WorktreeRecoveryRequired,
            "Git worktree inventory changed during publication",
        ));
    }
    let target = target.canonicalize().map_err(io_error)?;
    for item in &after {
        if item.branch.as_deref() == Some(session.target_ref.as_str()) {
            if item.path.canonicalize().map_err(io_error)? != target
                || item.head != prepared.candidate_commit
            {
                return Err(ClewError::new(
                    ErrorCode::WorktreeRecoveryRequired,
                    "published target worktree is inconsistent",
                ));
            }
            require_clean(&target)?;
        }
    }
    Ok(())
}

fn worktree_inventory(repository: &Path) -> Result<Vec<WorktreeView>, ClewError> {
    let output = Command::new("git")
        .args(["worktree", "list", "--porcelain", "-z"])
        .current_dir(repository)
        .stdin(Stdio::null())
        .output()
        .map_err(io_error)?;
    if !output.status.success() {
        return Err(invalid("Git worktree inventory is unavailable"));
    }
    parse_worktree_inventory(&output.stdout)
}

fn parse_worktree_inventory(bytes: &[u8]) -> Result<Vec<WorktreeView>, ClewError> {
    if !bytes.ends_with(b"\0\0") {
        return Err(invalid("Git worktree inventory is not record terminated"));
    }
    let mut inventory = Vec::new();
    let mut path = None::<PathBuf>;
    let mut head = None::<String>;
    let mut branch = None::<String>;
    for field in bytes.split(|byte| *byte == 0) {
        if field.is_empty() {
            if path.is_none() && head.is_none() && branch.is_none() {
                continue;
            }
            inventory.push(WorktreeView {
                path: path
                    .take()
                    .ok_or_else(|| invalid("Git worktree path is missing"))?,
                head: head
                    .take()
                    .ok_or_else(|| invalid("Git worktree HEAD is missing"))?,
                branch: branch.take(),
            });
            continue;
        }
        let field = std::str::from_utf8(field)
            .map_err(|_| invalid("Git worktree inventory is not UTF-8"))?;
        if let Some(value) = field.strip_prefix("worktree ") {
            if path.replace(PathBuf::from(value)).is_some() {
                return Err(invalid("Git worktree inventory repeats a path"));
            }
        } else if let Some(value) = field.strip_prefix("HEAD ") {
            if !git_oid(value) || head.replace(value.into()).is_some() {
                return Err(invalid("Git worktree inventory has an invalid HEAD"));
            }
        } else if let Some(value) = field.strip_prefix("branch ") {
            if !value.starts_with("refs/heads/") || branch.replace(value.into()).is_some() {
                return Err(invalid("Git worktree inventory has an invalid branch"));
            }
        } else if !matches!(field, "detached" | "bare" | "prunable" | "locked")
            && !field.starts_with("prunable ")
            && !field.starts_with("locked ")
        {
            return Err(invalid("Git worktree inventory has an unknown field"));
        }
    }
    if path.is_some() || head.is_some() || branch.is_some() {
        return Err(invalid("Git worktree inventory is not record terminated"));
    }
    if inventory.is_empty() {
        return Err(invalid("Git worktree inventory is empty"));
    }
    Ok(inventory)
}

fn git_oid(value: &str) -> bool {
    matches!(value.len(), 40 | 64)
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

pub fn recover(
    session: &SessionAuthority,
    prepared: &PreparedCandidateV2,
    candidate_root: &Path,
    approval: Option<&ConditionalPublicationApproval>,
) -> Result<Value, ClewError> {
    publish(session, prepared, candidate_root, approval)
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
        || prepared.semantic_generation.object_schema
            != crate::generation_service::READY_GENERATION_SET_SCHEMA
        || !digest(&prepared.semantic_generation_key)
        || !digest(&prepared.context_evidence_digest)
        || prepared.prepared_authority_digest != prepared_authority_digest(prepared)?
        || prepared.changed_files.is_empty()
        || !prepared
            .changed_files
            .windows(2)
            .all(|pair| pair[0] < pair[1])
        || prepared
            .qualified_obligations
            .windows(2)
            .any(|pair| pair[0].approval_id >= pair[1].approval_id)
        || prepared.qualified_obligations.iter().any(|item| {
            !digest(&item.record_digest)
                || canonical::hash(&item.record).ok().as_deref() != Some(&item.record_digest)
                || item.approval_id
                    != format!(
                        "{}:{}",
                        match item.source {
                            ObligationSource::Context => "context",
                            ObligationSource::Candidate => "candidate",
                        },
                        item.record_digest
                    )
        })
        || !digest(&prepared.diff.digest)
        || prepared.diff.over_limit == prepared.diff.patch.is_some()
        || prepared.diff.patch.as_ref().is_some_and(|patch| {
            patch.len() != prepared.diff.byte_size
                || canonical::hash_bytes(patch.as_bytes()) != prepared.diff.digest
        })
        || prepared
            .derived_outputs
            .windows(2)
            .any(|pair| pair[0].path >= pair[1].path)
        || prepared.derived_outputs.iter().any(|output| {
            !safe_path(&output.path)
                || !digest(&output.content_digest)
                || output.path == ".git"
                || output.path.starts_with(".git/")
        })
    {
        return Err(ClewError::new(
            ErrorCode::PreconditionFailed,
            "prepared candidate authority differs from the session",
        ));
    }
    Ok(())
}

fn validate_conditional_approval(
    session: &SessionAuthority,
    prepared: &PreparedCandidateV2,
    approval: &ConditionalPublicationApproval,
) -> Result<(), ClewError> {
    if !prepared.publication_blocked
        || !prepared.conditional_publish_eligible
        || approval.schema != "codeclew-conditional-publication-approval/1.0"
        || approval.mode != "ACKNOWLEDGED_UNSURE"
        || approval.session_authority_digest != session.authority_digest
        || approval.context_id != prepared.context_id
        || approval.context_evidence_digest != prepared.context_evidence_digest
        || approval.plan_id != prepared.plan_id
        || approval.obligations != prepared.qualified_obligations
        || approval.candidate_commit != prepared.candidate_commit
        || approval.candidate_snapshot != prepared.candidate_snapshot
        || approval.changed_files != prepared.changed_files
        || approval.validation_evidence != prepared.validation_evidence
        || approval.prepared_authority_digest != prepared.prepared_authority_digest
        || approval.run_id.is_empty()
        || !digest(&approval.request_digest)
    {
        return Err(ClewError::new(
            ErrorCode::PreconditionFailed,
            "conditional approval differs from prepared publication authority",
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
            ValidationLauncher::Python => "python3",
        };
        let mut command = Command::new(executable);
        command
            .args(&step.args)
            .current_dir(root)
            .env("GIT_TERMINAL_PROMPT", "0")
            .stdin(Stdio::null());
        isolate_controller_authority(&mut command)?;
        let output = command.output().map_err(io_error)?;
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
                    | ValidationLauncher::Maven
                    | ValidationLauncher::Python => ErrorCode::TestFailed,
                },
                "candidate validation failed; inspect private run evidence",
            ));
        }
    }
    Ok(evidence)
}

fn changed_paths(root: &Path) -> Result<BTreeSet<String>, ClewError> {
    let mut arguments = vec![
        "status",
        "--porcelain=v1",
        "-z",
        "--untracked-files=all",
        "--",
        ".",
    ];
    arguments.extend_from_slice(&LEGACY_EXCLUDES);
    nul_paths(root, &arguments)
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
    let original_permissions = match fs::symlink_metadata(path) {
        Ok(metadata) => Some(metadata.permissions()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(io_error(error)),
    };
    let temporary = parent.join(format!(".codeclew-edit-{}", uuid::Uuid::new_v4()));
    let mut options = OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    options.mode(0o600);
    let mut file = options.open(&temporary).map_err(io_error)?;
    file.write_all(bytes).map_err(io_error)?;
    file.sync_all().map_err(io_error)?;
    fs::rename(&temporary, path).map_err(io_error)?;
    if let Some(permissions) = original_permissions {
        fs::set_permissions(path, permissions).map_err(io_error)?;
    } else {
        #[cfg(unix)]
        fs::set_permissions(path, fs::Permissions::from_mode(0o644)).map_err(io_error)?;
    }
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
        && crate::text_authority::is_nfc(value)
        && !value.starts_with('/')
        && !value.contains('\0')
        && !value.split('/').any(|component| {
            component.is_empty()
                || component == "."
                || component == ".."
                || component == ".semantic-thread"
        })
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
    fn mutation_profiles_require_their_native_validator() {
        let plan = |launcher: &str, args: Value| {
            validate_plan_value(&json!({
                "schema":PLAN_V2_SCHEMA,
                "operations":[{
                    "kind":"DELETE_FILE",
                    "opId":"one",
                    "target":{"fileId":"source.txt","contentRef":reference()}
                }],
                "validation":[{"launcher":launcher,"args":args}],
            }))
            .unwrap()
        };
        assert!(
            require_profile_validation(
                MutationProfile::Kotlin24Gradle,
                &plan("GRADLE", json!(["test"])),
            )
            .is_ok()
        );
        assert!(
            require_profile_validation(
                MutationProfile::RustSyntax,
                &plan("CARGO", json!(["test"])),
            )
            .is_ok()
        );
        assert!(
            require_profile_validation(
                MutationProfile::PythonSyntax,
                &plan("PYTHON", json!(["-m", "unittest"])),
            )
            .is_ok()
        );
        assert!(
            require_profile_validation(
                MutationProfile::PythonSyntax,
                &plan("PYTHON", json!(["-c", "pass"])),
            )
            .is_err()
        );
        assert!(
            require_profile_validation(
                MutationProfile::RustSyntax,
                &plan("GRADLE", json!(["test"])),
            )
            .is_err()
        );
    }

    #[test]
    fn mutation_profiles_keep_unqualified_kotlin_read_only() {
        let versions = |value: Value| value.as_object().unwrap().clone();
        assert_eq!(
            mutation_profile_for(
                SessionLanguage::Kotlin,
                &[":/main".into()],
                &versions(json!({":/main":"2.4.10"})),
                true,
            )
            .unwrap(),
            MutationProfile::Kotlin24Gradle
        );
        for compiler in ["2.4.0", "2.3.0"] {
            assert!(
                mutation_profile_for(
                    SessionLanguage::Kotlin,
                    &[":/main".into()],
                    &versions(json!({":/main":compiler})),
                    true,
                )
                .is_err()
            );
        }
        assert!(
            mutation_profile_for(
                SessionLanguage::Kotlin,
                &[":/main".into()],
                &versions(json!({":/main":"2.4.10"})),
                false,
            )
            .is_err()
        );
        assert_eq!(
            mutation_profile_for(
                SessionLanguage::Python,
                &["python:.#src".into()],
                &versions(json!({"python:.#src":PYTHON_GRAMMAR_AUTHORITY})),
                false,
            )
            .unwrap(),
            MutationProfile::PythonSyntax
        );
        assert_eq!(
            mutation_profile_for(
                SessionLanguage::Rust,
                &["cargo:Cargo.toml#demo#lib#demo".into()],
                &versions(json!({
                    "cargo:Cargo.toml#demo#lib#demo":"rustc 1.92.0 (qualified)\nbinary: rustc"
                })),
                false,
            )
            .unwrap(),
            MutationProfile::RustSyntax
        );
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
        #[cfg(unix)]
        fs::set_permissions(root.path().join("A.kt"), fs::Permissions::from_mode(0o755)).unwrap();
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
        #[cfg(unix)]
        assert_eq!(
            fs::symlink_metadata(root.path().join("A.kt"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o755
        );
    }

    #[test]
    fn plan_preflight_rejects_embedded_diff_prefixes() {
        let plan = json!({
            "schema":PLAN_V2_SCHEMA,
            "operations":[{
                "kind":"REPLACE_TEXT",
                "opId":"replace",
                "target":{"fileId":"A.kt","contentRef":reference()},
                "oldText":"before",
                "newText":"first\n+second\n+third\n+fourth",
            }],
            "validation":[{"launcher":"GRADLE","args":["test"]}],
        });
        let error = validate_plan_value(&plan).unwrap_err();
        assert_eq!(error.code, ErrorCode::InvalidInput);
        assert!(error.message.contains("unified-diff"));
    }

    #[test]
    fn identical_multi_compilation_obligations_are_deduplicated() {
        let obligation = qualify_obligation(
            ObligationSource::Candidate,
            json!({
                "code":"VERIFY_PYTHON_RUNTIME_IMPORTS_AND_TYPES",
                "publicationBlocking":true,
                "subject":["scope"]
            }),
        )
        .unwrap();
        let normalized =
            normalize_qualified_obligations(vec![obligation.clone(), obligation.clone()]).unwrap();
        assert_eq!(normalized.as_slice(), std::slice::from_ref(&obligation));

        let mut conflicting = obligation.clone();
        conflicting.record["subject"] = json!(["different-scope"]);
        assert!(normalize_qualified_obligations(vec![obligation, conflicting]).is_err());
    }

    #[test]
    fn legacy_state_is_neither_a_plan_target_nor_a_cleanliness_input() {
        assert!(!safe_path(".semantic-thread/private"));
        assert!(!safe_path("src/.semantic-thread/private"));

        let repository = tempfile::tempdir().unwrap();
        git_status(
            Command::new("git")
                .args(["init", "-q", "-b", "main"])
                .current_dir(repository.path()),
            "test repository init failed",
        )
        .unwrap();
        fs::write(repository.path().join("README.md"), b"fixture\n").unwrap();
        git_status(
            Command::new("git")
                .args(["add", "README.md"])
                .current_dir(repository.path()),
            "test repository add failed",
        )
        .unwrap();
        git_status(
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
                .current_dir(repository.path()),
            "test repository commit failed",
        )
        .unwrap();

        let root_legacy = repository.path().join(".semantic-thread");
        let nested_legacy = repository.path().join("src/.semantic-thread");
        fs::create_dir_all(&root_legacy).unwrap();
        fs::create_dir_all(&nested_legacy).unwrap();
        fs::write(root_legacy.join("private"), b"ignored").unwrap();
        fs::write(nested_legacy.join("private"), b"ignored").unwrap();
        #[cfg(unix)]
        {
            fs::set_permissions(&root_legacy, fs::Permissions::from_mode(0o000)).unwrap();
            fs::set_permissions(&nested_legacy, fs::Permissions::from_mode(0o000)).unwrap();
        }
        let clean = require_clean(repository.path());
        #[cfg(unix)]
        {
            fs::set_permissions(&root_legacy, fs::Permissions::from_mode(0o700)).unwrap();
            fs::set_permissions(&nested_legacy, fs::Permissions::from_mode(0o700)).unwrap();
        }
        clean.unwrap();

        fs::write(repository.path().join("README.md"), b"changed\n").unwrap();
        assert_eq!(
            require_clean(repository.path()).unwrap_err().code,
            ErrorCode::PreconditionFailed
        );
    }

    #[test]
    fn parses_nul_delimited_worktree_authority_without_path_guessing() {
        let first = "a".repeat(40);
        let second = "b".repeat(40);
        let bytes = format!(
            "worktree /repo\0HEAD {first}\0branch refs/heads/main\0\0worktree /repo/candidate\0HEAD {second}\0detached\0\0"
        );

        let inventory = parse_worktree_inventory(bytes.as_bytes()).unwrap();

        assert_eq!(inventory.len(), 2);
        assert_eq!(inventory[0].path, Path::new("/repo"));
        assert_eq!(inventory[0].branch.as_deref(), Some("refs/heads/main"));
        assert_eq!(inventory[1].path, Path::new("/repo/candidate"));
        assert_eq!(inventory[1].branch, None);
    }

    #[test]
    fn worktree_inventory_rejects_unknown_or_unterminated_records() {
        let oid = "a".repeat(40);
        assert!(
            parse_worktree_inventory(
                format!("worktree /repo\0HEAD {oid}\0surprise\0\0").as_bytes()
            )
            .is_err()
        );
        assert!(
            parse_worktree_inventory(format!("worktree /repo\0HEAD {oid}\0").as_bytes()).is_err()
        );
    }

    #[test]
    fn prepared_candidate_refuses_single_generation_cas_authority() {
        let digest = |character: char| format!("sha256:{}", character.to_string().repeat(64));
        let session = SessionAuthority {
            schema: crate::session::SESSION_SCHEMA.into(),
            authority_digest: digest('a'),
            session_id: format!("session:{}", uuid::Uuid::new_v4()),
            repository_key: "a".repeat(64),
            base_revision: "1".repeat(40),
            target_ref: "refs/heads/main".into(),
            target_oid: "1".repeat(40),
            runtime_key: digest('b'),
            runtime_mode: crate::runtime::RuntimeMode::Development,
            language: crate::session::SessionLanguage::Kotlin,
            compilations: vec![":/main".into()],
            generation_jobs: None,
            model_cache_policy: crate::session::ModelCachePolicy::NonCacheable,
            model_cache_authority: None,
            created_unix_ms: 1,
        };
        let mut prepared = PreparedCandidateV2 {
            schema: PREPARED_V2_SCHEMA.into(),
            session_id: session.session_id.clone(),
            context_id: "context:test".into(),
            context_evidence_digest: digest('9'),
            plan_id: "plan:test".into(),
            base_revision: session.base_revision.clone(),
            target_ref: session.target_ref.clone(),
            target_oid: session.target_oid.clone(),
            candidate_commit: "2".repeat(40),
            candidate_snapshot: CasObject {
                schema: crate::cas::CAS_OBJECT_SCHEMA.into(),
                object_schema: crate::repository_snapshot::SNAPSHOT_SCHEMA.into(),
                digest: digest('c'),
                size: 1,
            },
            semantic_generation: CasObject {
                schema: crate::cas::CAS_OBJECT_SCHEMA.into(),
                object_schema: crate::generation_service::READY_GENERATION_SET_SCHEMA.into(),
                digest: digest('d'),
                size: 1,
            },
            semantic_generation_key: digest('e'),
            changed_files: vec!["A.kt".into()],
            validation_evidence: Vec::new(),
            qualified_obligations: Vec::new(),
            conditional_publish_eligible: false,
            diff: CandidateDiff {
                digest: canonical::hash_bytes(b""),
                byte_size: 0,
                over_limit: false,
                patch: Some(String::new()),
            },
            derived_outputs: Vec::new(),
            prepared_authority_digest: String::new(),
            publication_blocked: false,
        };
        prepared.prepared_authority_digest = prepared_authority_digest(&prepared).unwrap();
        validate_prepared(&session, &prepared).unwrap();
        prepared.semantic_generation.object_schema =
            crate::generation_service::READY_GENERATION_SCHEMA.into();
        assert_eq!(
            validate_prepared(&session, &prepared).unwrap_err().code,
            ErrorCode::PreconditionFailed
        );
    }

    #[test]
    fn conditional_approval_is_exact_content_bound_and_tamper_evident() {
        let digest = |character: char| format!("sha256:{}", character.to_string().repeat(64));
        let session = SessionAuthority {
            schema: crate::session::SESSION_SCHEMA.into(),
            authority_digest: digest('a'),
            session_id: format!("session:{}", uuid::Uuid::new_v4()),
            repository_key: "a".repeat(64),
            base_revision: "1".repeat(40),
            target_ref: "refs/heads/main".into(),
            target_oid: "1".repeat(40),
            runtime_key: digest('b'),
            runtime_mode: crate::runtime::RuntimeMode::Development,
            language: crate::session::SessionLanguage::Kotlin,
            compilations: vec![":/main".into()],
            generation_jobs: None,
            model_cache_policy: crate::session::ModelCachePolicy::NonCacheable,
            model_cache_authority: None,
            created_unix_ms: 1,
        };
        let obligation = qualify_obligation(
            ObligationSource::Context,
            json!({
                "code":"VERIFY_QUERY_SELECTION",
                "id":"verify-query-selection",
                "publicationBlocking":true,
                "requiredCheckSet":["confirm exact declarations and tests"],
                "subject":["total"]
            }),
        )
        .unwrap();
        let mut prepared = PreparedCandidateV2 {
            schema: PREPARED_V2_SCHEMA.into(),
            session_id: session.session_id.clone(),
            context_id: "context:test".into(),
            context_evidence_digest: digest('9'),
            plan_id: "plan:test".into(),
            base_revision: session.base_revision.clone(),
            target_ref: session.target_ref.clone(),
            target_oid: session.target_oid.clone(),
            candidate_commit: "2".repeat(40),
            candidate_snapshot: CasObject {
                schema: crate::cas::CAS_OBJECT_SCHEMA.into(),
                object_schema: crate::repository_snapshot::SNAPSHOT_SCHEMA.into(),
                digest: digest('c'),
                size: 1,
            },
            semantic_generation: CasObject {
                schema: crate::cas::CAS_OBJECT_SCHEMA.into(),
                object_schema: crate::generation_service::READY_GENERATION_SET_SCHEMA.into(),
                digest: digest('d'),
                size: 1,
            },
            semantic_generation_key: digest('e'),
            changed_files: vec!["src/A.kt".into()],
            validation_evidence: vec![ValidationEvidence {
                launcher: ValidationLauncher::Gradle,
                args_digest: digest('f'),
                output_digest: digest('8'),
                success: true,
            }],
            qualified_obligations: vec![obligation.clone()],
            conditional_publish_eligible: true,
            diff: CandidateDiff {
                digest: canonical::hash_bytes(b"patch"),
                byte_size: 5,
                over_limit: false,
                patch: Some("patch".into()),
            },
            derived_outputs: Vec::new(),
            prepared_authority_digest: String::new(),
            publication_blocked: true,
        };
        prepared.prepared_authority_digest = prepared_authority_digest(&prepared).unwrap();

        assert!(conditional_approval(&session, &prepared, "run:test", &digest('7'), &[],).is_err());
        let approval = conditional_approval(
            &session,
            &prepared,
            "run:test",
            &digest('7'),
            std::slice::from_ref(&obligation.approval_id),
        )
        .unwrap();
        validate_conditional_approval(&session, &prepared, &approval).unwrap();

        let mut tampered = approval.clone();
        tampered.changed_files.push("src/Unexpected.kt".into());
        assert!(validate_conditional_approval(&session, &prepared, &tampered).is_err());
        assert!(
            conditional_approval(
                &session,
                &prepared,
                "run:test",
                &digest('7'),
                &[obligation.approval_id.clone(), obligation.approval_id],
            )
            .is_err()
        );
    }

    #[test]
    fn candidate_commit_uses_generic_identity() {
        let repository = tempfile::tempdir().unwrap();
        git_status(
            Command::new("git")
                .args(["init", "-q", "-b", "main"])
                .current_dir(repository.path()),
            "test init failed",
        )
        .unwrap();
        git_status(
            Command::new("git")
                .args(["config", "user.name", "Private Developer"])
                .current_dir(repository.path()),
            "test Git name setup failed",
        )
        .unwrap();
        git_status(
            Command::new("git")
                .args(["config", "user.email", "private@example.invalid"])
                .current_dir(repository.path()),
            "test Git email setup failed",
        )
        .unwrap();
        fs::write(repository.path().join("tracked"), b"candidate\n").unwrap();
        git_status(
            Command::new("git")
                .args(["add", "tracked"])
                .current_dir(repository.path()),
            "test add failed",
        )
        .unwrap();
        commit_candidate(repository.path(), "plan:generic-identity").unwrap();
        assert_eq!(
            git(
                repository.path(),
                &["show", "-s", "--format=%an <%ae>|%cn <%ce>", "HEAD"]
            )
            .unwrap(),
            "Codeclew <noreply@example.invalid>|Codeclew <noreply@example.invalid>"
        );
    }

    #[test]
    fn derived_output_cleanup_requires_an_exact_post_process_manifest() {
        let repository = tempfile::tempdir().unwrap();
        git_status(
            Command::new("git")
                .args(["init", "-q", "-b", "main"])
                .current_dir(repository.path()),
            "test init failed",
        )
        .unwrap();
        fs::write(repository.path().join(".gitignore"), b"/build/\n").unwrap();
        fs::write(repository.path().join("A.kt"), b"fun answer() = 42\n").unwrap();
        git_status(
            Command::new("git")
                .args(["add", "."])
                .current_dir(repository.path()),
            "test add failed",
        )
        .unwrap();
        commit_candidate(repository.path(), "plan:base").unwrap();
        fs::create_dir(repository.path().join("build")).unwrap();
        let output = repository.path().join("build/result.bin");
        fs::write(&output, b"validated").unwrap();
        let manifest = capture_derived_outputs(repository.path()).unwrap();
        assert_eq!(manifest.len(), 1);

        fs::write(&output, b"tampered").unwrap();
        assert!(verify_exact_derived_outputs(repository.path(), &manifest).is_err());
        fs::write(&output, b"validated").unwrap();
        remove_exact_derived_outputs(repository.path(), &manifest).unwrap();
        assert!(!repository.path().join("build").exists());
    }

    #[test]
    fn recovery_accepts_only_one_direct_candidate_commit() {
        let root = tempfile::tempdir().unwrap();
        let candidate_root = root.path().join("candidate");
        let worktree = candidate_root.join("worktree");
        fs::create_dir_all(&worktree).unwrap();
        git_status(
            Command::new("git")
                .args(["init", "-q", "-b", "main"])
                .current_dir(&worktree),
            "test init failed",
        )
        .unwrap();
        fs::write(worktree.join("A.kt"), b"one\n").unwrap();
        git_status(
            Command::new("git")
                .args(["add", "A.kt"])
                .current_dir(&worktree),
            "test add failed",
        )
        .unwrap();
        commit_candidate(&worktree, "plan:base").unwrap();
        let base = git(&worktree, &["rev-parse", "HEAD"]).unwrap();
        let session = SessionAuthority {
            schema: crate::session::SESSION_SCHEMA.into(),
            authority_digest: format!("sha256:{}", "a".repeat(64)),
            session_id: format!("session:{}", uuid::Uuid::new_v4()),
            repository_key: "a".repeat(64),
            base_revision: base,
            target_ref: "refs/heads/main".into(),
            target_oid: "b".repeat(40),
            runtime_key: format!("sha256:{}", "c".repeat(64)),
            runtime_mode: crate::runtime::RuntimeMode::Development,
            language: crate::session::SessionLanguage::Kotlin,
            compilations: vec![":/main".into()],
            generation_jobs: None,
            model_cache_policy: crate::session::ModelCachePolicy::NonCacheable,
            model_cache_authority: None,
            created_unix_ms: 1,
        };
        fs::write(worktree.join("A.kt"), b"two\n").unwrap();
        git_status(
            Command::new("git")
                .args(["add", "A.kt"])
                .current_dir(&worktree),
            "test add failed",
        )
        .unwrap();
        commit_candidate(&worktree, "plan:candidate").unwrap();
        let candidate = recoverable_candidate_commit(&session, &candidate_root)
            .unwrap()
            .unwrap();
        assert_eq!(candidate, git(&worktree, &["rev-parse", "HEAD"]).unwrap());

        fs::write(worktree.join("A.kt"), b"three\n").unwrap();
        git_status(
            Command::new("git")
                .args(["add", "A.kt"])
                .current_dir(&worktree),
            "test add failed",
        )
        .unwrap();
        commit_candidate(&worktree, "plan:unexpected-second-commit").unwrap();
        assert!(recoverable_candidate_commit(&session, &candidate_root).is_err());
    }
}
