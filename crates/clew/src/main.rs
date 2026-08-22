use clap::{Args, Parser, Subcommand, ValueEnum};
use clew::canonical;
use clew::error::{ClewError, ErrorCode};
use clew::session::{
    ModelCachePolicy, RunRecord, RunStatus, SessionAuthority, bounded_context_stdout,
};
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::process::{ExitCode, Stdio};

#[derive(Parser)]
#[command(
    name = "clew",
    version,
    about = "Codeclew managed semantic change runtime"
)]
struct Cli {
    #[arg(long, global = true)]
    json: bool,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    Session {
        #[command(subcommand)]
        command: SessionCommand,
    },
    Context {
        #[command(subcommand)]
        command: ContextCommand,
    },
    Plan {
        #[command(subcommand)]
        command: PlanCommand,
    },
    #[command(name = "task-run")]
    TaskRun {
        #[command(subcommand)]
        command: TaskRunCommand,
    },
    #[command(name = "__task-run-execute", hide = true)]
    InternalTaskRunExecute(InternalTaskRunArgs),
}

#[derive(Subcommand)]
enum SessionCommand {
    Open(SessionOpenArgs),
    Inspect(SessionIdArgs),
    Publish(SessionRunArgs),
    Recover(SessionRunArgs),
}

#[derive(Subcommand)]
enum ContextCommand {
    Create(ContextCreateArgs),
    Expand(ContextExpandArgs),
}

#[derive(Subcommand)]
enum PlanCommand {
    Validate(PlanValidateArgs),
}

#[derive(Subcommand)]
enum TaskRunCommand {
    Start(TaskRunStartArgs),
    Status(RunIdArgs),
    Resume(RunIdArgs),
    Cancel(RunIdArgs),
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum ModelCachePolicyArg {
    NonCacheable,
    TrackedManifest,
    SealedExternal,
}

#[derive(Args)]
struct SessionOpenArgs {
    #[arg(long)]
    repo: PathBuf,
    #[arg(long)]
    target_ref: String,
    #[arg(long, default_value = ":/main")]
    compilation: String,
    #[arg(long, value_enum, default_value_t = ModelCachePolicyArg::NonCacheable)]
    model_cache: ModelCachePolicyArg,
}

#[derive(Args)]
struct SessionIdArgs {
    #[arg(long)]
    session: String,
}

#[derive(Args)]
struct SessionRunArgs {
    #[arg(long)]
    session: String,
    #[arg(long)]
    run: String,
}

#[derive(Args)]
struct ContextCreateArgs {
    #[arg(long)]
    session: String,
    #[arg(long, default_value = "")]
    intent: String,
    #[arg(long = "term", required = true)]
    terms: Vec<String>,
    #[arg(long, default_value_t = 2)]
    max_roots: usize,
}

#[derive(Args)]
struct ContextExpandArgs {
    #[arg(long)]
    session: String,
    #[arg(long = "from")]
    context: String,
    #[arg(long = "term", required = true)]
    terms: Vec<String>,
    #[arg(long)]
    intent: Option<String>,
    #[arg(long, default_value_t = 4)]
    max_roots: usize,
}

#[derive(Args)]
struct PlanValidateArgs {
    #[arg(long)]
    session: String,
    #[arg(long)]
    context: String,
    #[arg(long)]
    plan: PathBuf,
}

#[derive(Args)]
struct TaskRunStartArgs {
    #[arg(long)]
    session: String,
    #[arg(long)]
    context: String,
    #[arg(long)]
    plan: String,
}

#[derive(Args)]
struct RunIdArgs {
    #[arg(long)]
    run: String,
}

#[derive(Args)]
struct InternalTaskRunArgs {
    #[arg(long)]
    run: String,
}

fn main() -> ExitCode {
    let started = std::time::Instant::now();
    let result = run(Cli::parse());
    eprintln!(
        "{}",
        String::from_utf8(
            canonical::bytes(&json!({
                "event":"request_completed",
                "durationMs":started.elapsed().as_millis(),
                "success":result.is_ok(),
            }))
            .unwrap_or_default()
        )
        .unwrap_or_else(|_| "{\"event\":\"request_completed\"}".into())
    );
    match result {
        Ok(value) => {
            println!(
                "{}",
                canonical::pretty(&value).unwrap_or_else(|_| "{}".into())
            );
            ExitCode::SUCCESS
        }
        Err(error) => {
            let code = exit_code(&error.code);
            println!(
                "{}",
                canonical::pretty(&json!({"schema":"codeclew-error/2.0","error":error}))
                    .unwrap_or_else(|_| "{}".into())
            );
            ExitCode::from(code)
        }
    }
}

fn run(cli: Cli) -> Result<Value, ClewError> {
    match cli.command {
        Command::Session {
            command: SessionCommand::Open(args),
        } => {
            let policy = match args.model_cache {
                ModelCachePolicyArg::NonCacheable => ModelCachePolicy::NonCacheable,
                ModelCachePolicyArg::TrackedManifest => ModelCachePolicy::TrackedManifest,
                ModelCachePolicyArg::SealedExternal => ModelCachePolicy::SealedExternal,
            };
            let session = SessionAuthority::open(
                &absolute(&args.repo)?,
                &args.target_ref,
                &args.compilation,
                policy,
            )?;
            Ok(json!({"schema":"codeclew-session-open/2.0","status":"OPEN","session":session}))
        }
        Command::Session {
            command: SessionCommand::Inspect(args),
        } => {
            let (session, _) = SessionAuthority::load(&args.session)?;
            Ok(json!({"schema":"codeclew-session-inspect/2.0","session":session}))
        }
        Command::Context {
            command: ContextCommand::Create(args),
        } => {
            let (session, _) = SessionAuthority::load(&args.session)?;
            let (projection, evidence) = clew::context_v2::create(
                &session,
                &args.intent,
                &args.terms,
                args.max_roots,
                None,
            )?;
            bounded_context_stdout(&session.store_context(
                None,
                args.intent,
                args.terms,
                projection,
                evidence,
            )?)
        }
        Command::Context {
            command: ContextCommand::Expand(args),
        } => {
            let (session, _) = SessionAuthority::load(&args.session)?;
            let parent = session.load_context(&args.context)?;
            let additional_terms = args.terms;
            let mut terms = parent.terms.clone();
            terms.extend(additional_terms.iter().cloned());
            terms.sort();
            terms.dedup();
            let intent = args.intent.unwrap_or_else(|| parent.intent.clone());
            let (projection, evidence) = clew::context_v2::create(
                &session,
                &intent,
                &additional_terms,
                args.max_roots,
                Some(&parent),
            )?;
            bounded_context_stdout(&session.store_context(
                Some(args.context),
                intent,
                terms,
                projection,
                evidence,
            )?)
        }
        Command::Plan {
            command: PlanCommand::Validate(args),
        } => {
            let (session, _) = SessionAuthority::load(&args.session)?;
            let metadata = std::fs::symlink_metadata(&args.plan).map_err(io_error)?;
            if metadata.file_type().is_symlink()
                || !metadata.is_file()
                || metadata.len() as usize > clew::session::MAX_PLAN_BYTES
            {
                return Err(invalid("plan is missing, unsafe, or exceeds 1 MiB"));
            }
            let plan = session
                .validate_plan(&args.context, &std::fs::read(&args.plan).map_err(io_error)?)?;
            Ok(json!({
                "schema":"codeclew-plan-validation/2.0","status":"VALID",
                "sessionId":session.session_id,"contextId":args.context,
                "planId":plan.plan_id,"sourceDigest":plan.source_digest,
            }))
        }
        Command::TaskRun {
            command: TaskRunCommand::Start(args),
        } => start_task_run(&args.session, &args.context, &args.plan),
        Command::TaskRun {
            command: TaskRunCommand::Status(args),
        } => task_run_status(&args.run),
        Command::TaskRun {
            command: TaskRunCommand::Resume(args),
        } => resume_task_run(&args.run),
        Command::TaskRun {
            command: TaskRunCommand::Cancel(args),
        } => cancel_task_run(&args.run),
        Command::Session {
            command: SessionCommand::Publish(args),
        } => publish_task_run(&args.session, &args.run),
        Command::Session {
            command: SessionCommand::Recover(args),
        } => recover_task_run(&args.session, &args.run),
        Command::InternalTaskRunExecute(args) => execute_task_run(&args.run),
    }
}

fn start_task_run(session_id: &str, context_id: &str, plan_id: &str) -> Result<Value, ClewError> {
    let (session, _) = SessionAuthority::load(session_id)?;
    let record = RunRecord::created(&session, context_id, plan_id)?;
    if !record.create_once()? {
        return task_run_status(&record.run_id);
    }
    spawn_task_run(&record.run_id)?;
    task_run_status(&record.run_id)
}

fn spawn_task_run(run_id: &str) -> Result<(), ClewError> {
    #[cfg(unix)]
    use std::os::unix::fs::OpenOptionsExt;
    #[cfg(unix)]
    use std::os::unix::process::CommandExt;
    let state = clew::state::StateAuthority::process_default()?;
    let root = state.run_root(run_id)?;
    let mut options = std::fs::OpenOptions::new();
    options.create(true).append(true).write(true);
    #[cfg(unix)]
    options.mode(0o600);
    let stdout = options.open(root.join("stdout.log")).map_err(io_error)?;
    let stderr = options.open(root.join("stderr.log")).map_err(io_error)?;
    let mut command = std::process::Command::new(std::env::current_exe().map_err(io_error)?);
    command
        .args(["__task-run-execute", "--run", run_id])
        .stdin(Stdio::null())
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr));
    #[cfg(unix)]
    command.process_group(0);
    let child = command.spawn().map_err(io_error)?;
    let mut latest = RunRecord::load(run_id)?;
    if latest.status == RunStatus::Created {
        latest.process_id = Some(child.id());
        latest.process_start_token = process_start_token(child.id())?;
        latest.save()?;
    }
    Ok(())
}

fn task_run_status(run_id: &str) -> Result<Value, ClewError> {
    Ok(json!({"schema":"codeclew-task-run-status/2.0","run":RunRecord::load(run_id)?}))
}

fn resume_task_run(run_id: &str) -> Result<Value, ClewError> {
    let mut record = RunRecord::load(run_id)?;
    if matches!(
        record.status,
        RunStatus::ReadyToPublish
            | RunStatus::ValidatedConditional
            | RunStatus::Published
            | RunStatus::Publishing
    ) {
        return task_run_status(run_id);
    }
    if record.candidate_commit.is_some() {
        record.status = RunStatus::WorktreeRecoveryRequired;
        record.failure = Some(json!({"code":"WORKTREE_RECOVERY_REQUIRED",
            "message":"candidate exists but preparation did not reach a validated state"}));
        record.save()?;
        return task_run_status(run_id);
    }
    record.status = RunStatus::Created;
    record.failure = None;
    record.process_id = None;
    record.process_start_token = None;
    record.save()?;
    spawn_task_run(run_id)?;
    task_run_status(run_id)
}

fn cancel_task_run(run_id: &str) -> Result<Value, ClewError> {
    let mut record = RunRecord::load(run_id)?;
    if record.status == RunStatus::Cancelled {
        return task_run_status(run_id);
    }
    if !cancellation_allowed(record.status) {
        return Err(ClewError::new(
            ErrorCode::PreconditionFailed,
            "only a created or preparing run can be cancelled",
        ));
    }
    let process = record.process_id.zip(record.process_start_token.clone());
    record.status = RunStatus::Cancelled;
    record.failure = None;
    record.save()?;
    if let Some((pid, token)) = process {
        terminate_verified_process_group(pid, &token)?;
    }
    let mut latest = RunRecord::load(run_id)?;
    latest.status = RunStatus::Cancelled;
    latest.process_id = None;
    latest.process_start_token = None;
    latest.save()?;
    task_run_status(run_id)
}

fn cancellation_allowed(status: RunStatus) -> bool {
    matches!(status, RunStatus::Created | RunStatus::Preparing)
}

fn execute_task_run(run_id: &str) -> Result<Value, ClewError> {
    let mut record = RunRecord::load(run_id)?;
    if record.status != RunStatus::Created {
        return task_run_status(run_id);
    }
    record.status = RunStatus::Preparing;
    record.process_id = Some(std::process::id());
    record.process_start_token = process_start_token(std::process::id())?;
    record.failure = None;
    record.save()?;
    match prepare_task_run(&mut record) {
        Ok(value) => Ok(value),
        Err(error) => {
            record.status = if RunRecord::load(run_id)?.status == RunStatus::Cancelled {
                RunStatus::Cancelled
            } else if error.code == ErrorCode::WorktreeRecoveryRequired {
                RunStatus::WorktreeRecoveryRequired
            } else {
                RunStatus::Failed
            };
            record.failure = serde_json::to_value(&error).ok();
            record.process_id = None;
            record.process_start_token = None;
            let _ = record.save();
            Err(error)
        }
    }
}

fn prepare_task_run(record: &mut RunRecord) -> Result<Value, ClewError> {
    let (session, _) = SessionAuthority::load(&record.session_id)?;
    let context = session.load_context(&record.context_id)?;
    let plan = session.load_plan(&record.plan_id)?;
    if context.evidence.get("schema").and_then(Value::as_str)
        != Some("codeclew-bounded-context-evidence/2.0")
    {
        return Err(invalid("task run requires bounded context v2"));
    }
    let prepared =
        clew::task_run_v2::prepare(&session, &context, &plan, &record.candidate_root()?)?;
    let state = clew::state::StateAuthority::process_default()?;
    state.write_private_atomic(
        &state.run_root(&record.run_id)?.join("prepared-v2.json"),
        &canonical::bytes(&prepared).map_err(internal)?,
    )?;
    record.candidate_commit = Some(prepared.candidate_commit.clone());
    record.candidate_snapshot = Some(prepared.candidate_snapshot.clone());
    record.publication_blocked = prepared.publication_blocked;
    record.status = if RunRecord::load(&record.run_id)?.status == RunStatus::Cancelled {
        RunStatus::Cancelled
    } else if prepared.publication_blocked {
        RunStatus::ValidatedConditional
    } else {
        RunStatus::ReadyToPublish
    };
    record.process_id = None;
    record.process_start_token = None;
    record.save()?;
    Ok(json!({"schema":"codeclew-task-run-preparation/2.0","run":record,"candidate":prepared}))
}

fn publish_task_run(session_id: &str, run_id: &str) -> Result<Value, ClewError> {
    let (session, _) = SessionAuthority::load(session_id)?;
    let mut record = RunRecord::load(run_id)?;
    require_run_session(&record, session_id)?;
    if record.publication_blocked || record.status == RunStatus::ValidatedConditional {
        return Err(ClewError::new(
            ErrorCode::IncompleteSemanticAnalysis,
            "conditional run cannot be published; create a new context, plan, and run",
        ));
    }
    if record.status == RunStatus::Published {
        return task_run_status(run_id);
    }
    if record.status != RunStatus::ReadyToPublish {
        return Err(ClewError::new(
            ErrorCode::PreconditionFailed,
            "run is not ready to publish",
        ));
    }
    let state = clew::state::StateAuthority::process_default()?;
    let prepared: clew::task_run_v2::PreparedCandidateV2 = read_json(
        &state.run_root(run_id)?.join("prepared-v2.json"),
        16 * 1024 * 1024,
    )?;
    record.status = RunStatus::Publishing;
    record.save()?;
    match clew::task_run_v2::publish(&session, &prepared, &record.candidate_root()?) {
        Ok(publication) => {
            record.status = RunStatus::Published;
            record.final_commit = Some(prepared.candidate_commit.clone());
            record.failure = None;
            record.save()?;
            Ok(json!({"schema":"codeclew-session-publish-result/2.0",
                "run":record,"publication":publication}))
        }
        Err(error) => {
            record.status = if error.code == ErrorCode::WorktreeRecoveryRequired {
                RunStatus::WorktreeRecoveryRequired
            } else {
                RunStatus::ReadyToPublish
            };
            record.failure = serde_json::to_value(&error).ok();
            record.save()?;
            Err(error)
        }
    }
}

fn recover_task_run(session_id: &str, run_id: &str) -> Result<Value, ClewError> {
    let (session, _) = SessionAuthority::load(session_id)?;
    let mut record = RunRecord::load(run_id)?;
    require_run_session(&record, session_id)?;
    if record.status == RunStatus::Published {
        return task_run_status(run_id);
    }
    if !matches!(
        record.status,
        RunStatus::Publishing | RunStatus::WorktreeRecoveryRequired
    ) {
        return Err(ClewError::new(
            ErrorCode::PreconditionFailed,
            "only a publishing or recovery-required run can be recovered",
        ));
    }
    let state = clew::state::StateAuthority::process_default()?;
    let prepared: clew::task_run_v2::PreparedCandidateV2 = read_json(
        &state.run_root(run_id)?.join("prepared-v2.json"),
        16 * 1024 * 1024,
    )?;
    match clew::task_run_v2::recover(&session, &prepared, &record.candidate_root()?) {
        Ok(recovery) => {
            record.status = RunStatus::Published;
            record.final_commit = Some(prepared.candidate_commit.clone());
            record.failure = None;
            record.save()?;
            Ok(json!({"schema":"codeclew-session-recover-result/2.0",
                "run":record,"recovery":recovery}))
        }
        Err(error) => {
            record.status = RunStatus::WorktreeRecoveryRequired;
            record.failure = serde_json::to_value(&error).ok();
            record.save()?;
            Err(error)
        }
    }
}

fn require_run_session(record: &RunRecord, session_id: &str) -> Result<(), ClewError> {
    if record.session_id != session_id {
        return Err(ClewError::new(
            ErrorCode::PreconditionFailed,
            "run belongs to another session",
        ));
    }
    Ok(())
}

fn process_start_token(pid: u32) -> Result<Option<String>, ClewError> {
    let output = std::process::Command::new("ps")
        .args(["-o", "lstart=", "-p", &pid.to_string()])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .map_err(io_error)?;
    if !output.status.success() {
        return Ok(None);
    }
    let token = String::from_utf8(output.stdout)
        .map_err(|_| internal("process identity is not UTF-8"))?
        .trim()
        .to_owned();
    Ok((!token.is_empty()).then_some(token))
}

fn process_is_active(pid: u32, expected_start: &str) -> Result<bool, ClewError> {
    if process_start_token(pid)?.as_deref() != Some(expected_start) {
        return Ok(false);
    }
    let output = std::process::Command::new("ps")
        .args(["-o", "stat=", "-p", &pid.to_string()])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .map_err(io_error)?;
    if !output.status.success() {
        return Ok(false);
    }
    let status =
        String::from_utf8(output.stdout).map_err(|_| internal("process status is not UTF-8"))?;
    Ok(!status.trim().is_empty() && !status.trim_start().starts_with('Z'))
}

#[cfg(unix)]
fn terminate_verified_process_group(pid: u32, expected_start: &str) -> Result<(), ClewError> {
    let pid = i32::try_from(pid)
        .ok()
        .filter(|pid| *pid > 1)
        .ok_or_else(|| invalid("run process id is invalid"))?;
    if !process_is_active(pid as u32, expected_start)? {
        return Err(ClewError::new(
            ErrorCode::PreconditionFailed,
            "run process identity changed before cancellation",
        ));
    }
    signal_group(pid, libc::SIGTERM)?;
    for _ in 0..40 {
        if !process_is_active(pid as u32, expected_start)? {
            return Ok(());
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    signal_group(pid, libc::SIGKILL)
}

#[cfg(unix)]
fn signal_group(pid: i32, signal: i32) -> Result<(), ClewError> {
    let result = unsafe { libc::kill(-pid, signal) };
    if result == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH) {
        Ok(())
    } else {
        Err(io_error(std::io::Error::last_os_error()))
    }
}

#[cfg(not(unix))]
fn terminate_verified_process_group(_pid: u32, _expected_start: &str) -> Result<(), ClewError> {
    Err(ClewError::new(
        ErrorCode::UnsupportedProjectConfiguration,
        "run cancellation requires Unix process-group authority",
    ))
}

fn read_json<T: DeserializeOwned>(path: &Path, limit: usize) -> Result<T, ClewError> {
    let metadata = std::fs::symlink_metadata(path).map_err(io_error)?;
    if metadata.file_type().is_symlink() || !metadata.is_file() || metadata.len() > limit as u64 {
        return Err(invalid("managed JSON is missing, unsafe, or oversized"));
    }
    serde_json::from_slice(&std::fs::read(path).map_err(io_error)?).map_err(parse_error)
}

fn absolute(path: &Path) -> Result<PathBuf, ClewError> {
    path.canonicalize().map_err(io_error)
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

fn exit_code(code: &ErrorCode) -> u8 {
    match code {
        ErrorCode::InvalidInput => 2,
        ErrorCode::SymbolNotFound | ErrorCode::ExpressionNotFound => 3,
        ErrorCode::StaleTarget
        | ErrorCode::StaleRequiresReslice
        | ErrorCode::ProjectModelChanged => 4,
        ErrorCode::AmbiguousTarget
        | ErrorCode::AmbiguousSymbol
        | ErrorCode::RwConflict
        | ErrorCode::WwConflict => 5,
        ErrorCode::ReplacementParseError
        | ErrorCode::TypeMismatch
        | ErrorCode::NewDiagnostics
        | ErrorCode::CompileFailed
        | ErrorCode::TestFailed => 6,
        ErrorCode::WorkerCrashed | ErrorCode::WorkerProtocolMismatch => 7,
        _ => 1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn removed_entrypoints_are_unparseable() {
        for removed in [
            "doctor",
            "project",
            "index",
            "resolve",
            "thread",
            "task-apply",
        ] {
            assert!(Cli::try_parse_from(["clew", removed]).is_err());
        }
    }

    #[test]
    fn recovery_and_cancellation_are_explicit() {
        assert!(
            Cli::try_parse_from([
                "clew",
                "session",
                "recover",
                "--session",
                "session:authority",
                "--run",
                "run:request"
            ])
            .is_ok()
        );
        assert!(
            Cli::try_parse_from(["clew", "task-run", "cancel", "--run", "run:request"]).is_ok()
        );
        assert!(cancellation_allowed(RunStatus::Created));
        assert!(cancellation_allowed(RunStatus::Preparing));
        assert!(!cancellation_allowed(RunStatus::Publishing));
        assert!(!cancellation_allowed(RunStatus::Published));
        assert!(!cancellation_allowed(RunStatus::WorktreeRecoveryRequired));
    }

    #[cfg(unix)]
    #[test]
    fn cancellation_targets_only_verified_process_group() {
        use std::os::unix::process::CommandExt;
        let mut child = std::process::Command::new("sleep")
            .arg("30")
            .process_group(0)
            .spawn()
            .unwrap();
        let token = process_start_token(child.id()).unwrap().unwrap();
        terminate_verified_process_group(child.id(), &token).unwrap();
        assert!(child.wait().unwrap().code().is_none());
    }
}
