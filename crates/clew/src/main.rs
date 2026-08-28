use clap::{Args, Parser, Subcommand, ValueEnum};
use clew::canonical;
use clew::error::{ClewError, ErrorCode};
use clew::operations::{capabilities, doctor, support_summary};
use clew::runtime::RuntimeAuthority;
use clew::session::mission;
use clew::session::{
    ModelCachePolicy, RunRecord, RunStatus, SessionAuthority, SessionLanguage,
    bounded_context_stdout, validate_context_request,
};
use clew::thread::{ThreadAuthority, ThreadMemberRequest};
use clew::thread_context::{bounded_thread_context_stdout, create as create_thread_context};
use clew::workspace;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use std::fmt::Write as _;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{ExitCode, Stdio};

#[cfg(unix)]
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};

#[derive(Parser)]
#[command(
    name = "clew",
    version,
    about = "Codeclew managed semantic change runtime"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Print the exact product support matrix bound to the active runtime.
    Capabilities(CapabilitiesArgs),
    /// Check host, runtime, state, and optional target-repository readiness.
    Doctor(DoctorArgs),
    /// Update an installed macOS release. Source checkouts are updated with Git.
    Upgrade,
    Change {
        #[command(subcommand)]
        command: ChangeCommand,
    },
    Mission {
        #[command(subcommand)]
        command: MissionCommand,
    },
    Workspace {
        #[command(subcommand)]
        command: WorkspaceCommand,
    },
    Session {
        #[command(subcommand)]
        command: SessionCommand,
    },
    Context {
        #[command(subcommand)]
        command: ContextCommand,
    },
    Thread {
        #[command(subcommand)]
        command: ThreadCommand,
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
    Support {
        #[command(subcommand)]
        command: SupportCommand,
    },
    #[command(name = "__task-run-execute", hide = true)]
    InternalTaskRunExecute(InternalTaskRunArgs),
}

#[derive(Args)]
struct CapabilitiesArgs {
    /// Render a concise human-readable report instead of canonical JSON.
    #[arg(long)]
    human: bool,
}

#[derive(Subcommand)]
enum ChangeCommand {
    Open(ChangeOpenArgs),
    CheckFreshness(SessionIdArgs),
    Prepare(ChangePrepareArgs),
    Status(RunIdArgs),
    Publish(SessionPublishArgs),
    Recover(SessionRunArgs),
}

#[derive(Subcommand)]
enum MissionCommand {
    Open(MissionOpenArgs),
    Record(MissionRecordArgs),
    /// Create one immutable evidence-native record from current mission bindings.
    Develop(MissionDevelopArgs),
    /// Render a deterministic dossier or inspect one graph node's evidence.
    Dossier(MissionDossierArgs),
    Inspect(MissionIdArgs),
    Status(MissionIdArgs),
    Close(MissionIdArgs),
}

#[derive(Subcommand)]
enum WorkspaceCommand {
    /// Resolve an explicit private catalog into one deterministic authority.
    Open(WorkspaceOpenArgs),
    Inspect(WorkspaceIdArgs),
    /// Reuse the globally bounded multi-repository context engine.
    Context(WorkspaceContextArgs),
    /// Prepare every independently planned member before any ref can publish.
    Prepare(WorkspacePrepareArgs),
    /// Retain one provider-neutral runtime observation against prepared candidates.
    Observe(WorkspaceObserveArgs),
    /// Publish every prepared member in one pre-sealed roll-forward order.
    Publish(WorkspacePublishArgs),
    /// Resume the unpublished suffix of an interrupted workspace publication.
    Recover(WorkspaceRecoverArgs),
    Close(WorkspaceIdArgs),
}

#[derive(Subcommand)]
enum SupportCommand {
    /// Build an allowlist-only summary from a private Codeclew JSON artifact.
    Summarize(SupportSummarizeArgs),
}

#[derive(Subcommand)]
enum SessionCommand {
    Open(SessionOpenArgs),
    Inspect(SessionIdArgs),
    Close(SessionIdArgs),
    Abort(SessionIdArgs),
    Relocate(SessionRelocateArgs),
    Gc(SessionGcArgs),
    Publish(SessionPublishArgs),
    Recover(SessionRunArgs),
}

#[derive(Subcommand)]
enum ContextCommand {
    Create(ContextCreateArgs),
    Expand(ContextExpandArgs),
}

#[derive(Subcommand)]
enum ThreadCommand {
    Open(ThreadOpenArgs),
    Context(ThreadContextArgs),
    Callables(ThreadCallablesArgs),
    Flow(ThreadFlowArgs),
    Explain(ThreadExplainArgs),
    Render(ThreadRenderArgs),
    ExplanationStatus(ThreadExplanationStatusArgs),
    Impact(ThreadImpactArgs),
    Validate(ThreadValidateArgs),
    Close(ThreadIdArgs),
    Gc(ThreadIdArgs),
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

#[derive(Debug, Clone, Copy, ValueEnum)]
enum SessionLanguageArg {
    Java,
    Kotlin,
    Python,
    Rust,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum ThreadImpactSubjectKindArg {
    FullSymbol,
    CallableFamily,
    Token,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum ThreadFlowRootKindArg {
    FullSymbol,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum ThreadFlowDirectionArg {
    Downstream,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum ExplanationDetailArg {
    Summary,
    Scenario,
    Technical,
    Evidence,
    Compiler,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum ExplanationFormatArg {
    Json,
    Markdown,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum DossierFormatArg {
    Json,
    Markdown,
    Dot,
}

#[derive(Args)]
struct SessionOpenArgs {
    #[arg(long)]
    repo: PathBuf,
    #[arg(long)]
    target_ref: String,
    /// Exact language authority. Required for mixed-language repositories.
    #[arg(long, value_enum)]
    language: SessionLanguageArg,
    /// Exact build compilation authority (for example :/main or :app/main).
    /// Repeat the option to select multiple compilations.
    /// Deliberately required: guessing the root compilation makes session
    /// admission succeed and defers a deterministic model error to context
    /// creation in multi-project builds.
    #[arg(long, required = true)]
    compilation: Vec<String>,
    /// Explicit resource-aware generation concurrency; omitted means host-adaptive.
    #[arg(long)]
    generation_jobs: Option<usize>,
    #[arg(long, value_enum, default_value_t = ModelCachePolicyArg::NonCacheable)]
    model_cache: ModelCachePolicyArg,
    #[arg(long, requires = "model_cache")]
    external_build_state: Option<PathBuf>,
}

#[derive(Args)]
struct DoctorArgs {
    /// Optional target repository to check without opening a session.
    #[arg(long)]
    repo: Option<PathBuf>,
    /// Optional ref that must resolve to the checked-out HEAD.
    #[arg(long, requires = "repo")]
    target_ref: Option<String>,
    /// Render an actionable human-readable report instead of canonical JSON.
    #[arg(long)]
    human: bool,
}

#[derive(Args)]
struct SupportSummarizeArgs {
    /// Absolute caller-owned mode-0600 file containing one Codeclew JSON result.
    #[arg(long)]
    input: PathBuf,
}

#[derive(Args)]
struct ChangeOpenArgs {
    #[command(flatten)]
    session: SessionOpenArgs,
    #[arg(long)]
    intent: String,
    #[arg(long = "term", required = true)]
    terms: Vec<String>,
    #[arg(long, default_value_t = 2)]
    max_roots: usize,
}

#[derive(Args)]
struct ChangePrepareArgs {
    #[arg(long)]
    session: String,
    #[arg(long)]
    context: String,
    #[arg(long)]
    plan: PathBuf,
}

#[derive(Args)]
struct MissionOpenArgs {
    #[arg(long = "session", required = true)]
    sessions: Vec<String>,
    #[arg(long)]
    spec: PathBuf,
}

#[derive(Args)]
struct MissionRecordArgs {
    #[arg(long)]
    mission: String,
    #[arg(long)]
    session: String,
    #[arg(long)]
    context: Option<String>,
    #[arg(long)]
    plan: Option<String>,
    #[arg(long)]
    run: Option<String>,
}

#[derive(Args)]
struct MissionDevelopArgs {
    #[arg(long)]
    mission: String,
    /// Closed, canonical codeclew-development-record-input/1.0 JSON file.
    #[arg(long)]
    record: PathBuf,
}

#[derive(Args)]
struct MissionDossierArgs {
    #[arg(long)]
    mission: String,
    #[arg(long)]
    record: String,
    #[arg(long, value_enum, default_value_t = DossierFormatArg::Json)]
    format: DossierFormatArg,
    /// Return one node and only its node-specific evidence.
    #[arg(long)]
    node: Option<String>,
}

#[derive(Args)]
struct MissionIdArgs {
    #[arg(long)]
    mission: String,
}

#[derive(Args)]
struct WorkspaceOpenArgs {
    /// Closed, canonical codeclew-workspace-catalog-input/1.0 JSON file.
    #[arg(long)]
    catalog: PathBuf,
}

#[derive(Args)]
struct WorkspaceIdArgs {
    #[arg(long)]
    workspace: String,
}

#[derive(Args)]
struct WorkspaceContextArgs {
    #[arg(long)]
    workspace: String,
    #[arg(long)]
    intent: String,
    #[arg(long = "term", required = true)]
    terms: Vec<String>,
    #[arg(long, default_value_t = 2)]
    max_roots: usize,
}

#[derive(Args)]
struct WorkspacePrepareArgs {
    #[arg(long)]
    workspace: String,
    /// Closed canonical codeclew-workspace-prepare-input/1.0 JSON file.
    #[arg(long)]
    request: PathBuf,
}

#[derive(Args)]
struct WorkspaceObserveArgs {
    #[arg(long)]
    workspace: String,
    /// Closed canonical codeclew-scenario-observation-input/1.0 JSON file.
    #[arg(long)]
    request: PathBuf,
    /// Private bounded raw provider evidence retained in CAS and never printed.
    #[arg(long)]
    evidence: PathBuf,
}

#[derive(Args)]
struct WorkspacePublishArgs {
    #[arg(long)]
    workspace: String,
    /// Closed canonical codeclew-workspace-publish-input/1.0 JSON file.
    #[arg(long)]
    request: PathBuf,
}

#[derive(Args)]
struct WorkspaceRecoverArgs {
    #[arg(long)]
    workspace: String,
    #[arg(long)]
    publication: String,
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
struct SessionPublishArgs {
    #[arg(long)]
    session: String,
    #[arg(long)]
    run: String,
    #[arg(long, requires = "prepared_authority_digest")]
    allow_conditional: bool,
    #[arg(long, requires = "allow_conditional")]
    prepared_authority_digest: Option<String>,
    #[arg(long = "acknowledge-obligation", requires = "allow_conditional")]
    acknowledge_obligations: Vec<String>,
}

#[derive(Args)]
struct SessionRelocateArgs {
    #[arg(long)]
    session: String,
    #[arg(long)]
    repo: PathBuf,
}

#[derive(Args)]
struct SessionGcArgs {
    #[arg(long)]
    session: String,
    #[arg(long)]
    force: bool,
}

#[derive(Args)]
struct ContextCreateArgs {
    #[arg(long)]
    session: String,
    #[arg(long)]
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
struct ThreadOpenArgs {
    /// Analysis-unit binding in ALIAS=SESSION_ID form. Repeat 2-8 times.
    #[arg(long = "member", required = true)]
    members: Vec<String>,
    /// Optional ALIAS=SERVICE_ALIAS override. Unspecified service aliases use
    /// the member alias.
    #[arg(long = "service-alias")]
    service_aliases: Vec<String>,
}

#[derive(Args)]
struct ThreadContextArgs {
    #[arg(long)]
    thread: String,
    #[arg(long)]
    intent: String,
    #[arg(long = "term", required = true)]
    terms: Vec<String>,
    #[arg(long, default_value_t = 2)]
    max_roots: usize,
}

#[derive(Args)]
struct ThreadCallablesArgs {
    #[arg(long)]
    thread: String,
    #[arg(long)]
    context: String,
    #[arg(long)]
    task_id: String,
    #[arg(long)]
    pair_id: String,
    #[arg(long)]
    provider: String,
    #[arg(long)]
    consumer: String,
    #[arg(long = "term", required = true)]
    terms: Vec<String>,
}

#[derive(Args)]
struct ThreadImpactArgs {
    #[arg(long)]
    thread: String,
    #[arg(long)]
    fact_set: String,
    #[arg(long)]
    pair_id: String,
    #[arg(long, value_enum)]
    subject_kind: ThreadImpactSubjectKindArg,
    #[arg(long)]
    subject: String,
    #[arg(long, required_if_eq("subject_kind", "full-symbol"))]
    member: Option<String>,
    #[arg(long)]
    declarations_only: bool,
    #[arg(long)]
    declaration_name_only: bool,
}

#[derive(Args)]
struct ThreadFlowArgs {
    #[arg(long)]
    thread: String,
    #[arg(long)]
    fact_set: String,
    #[arg(long)]
    pair_id: String,
    #[arg(long)]
    member: String,
    #[arg(long, value_enum)]
    root_kind: ThreadFlowRootKindArg,
    #[arg(long)]
    root: String,
    #[arg(long, value_enum)]
    direction: ThreadFlowDirectionArg,
    #[arg(long, default_value_t = 4)]
    max_depth: usize,
}

#[derive(Args)]
struct ThreadExplainArgs {
    #[arg(long)]
    thread: String,
    #[arg(long)]
    flow: String,
    /// Closed, canonical codeclew-explanation-claim-input/0.1 JSON file.
    #[arg(long)]
    claims: PathBuf,
}

#[derive(Args)]
struct ThreadRenderArgs {
    #[arg(long)]
    thread: String,
    #[arg(long)]
    explanation: String,
    #[arg(long, value_enum)]
    detail: ExplanationDetailArg,
    #[arg(long, value_enum)]
    format: ExplanationFormatArg,
}

#[derive(Args)]
struct ThreadExplanationStatusArgs {
    /// Thread that owns the immutable explanation bundle.
    #[arg(long)]
    thread: String,
    #[arg(long)]
    explanation: String,
    /// New thread snapshot used only as comparison authority.
    #[arg(long)]
    against_thread: String,
    #[arg(long)]
    against_fact_set: String,
    #[arg(long)]
    against_flow: String,
    /// Total old=against mapping for the selected provider/consumer pair.
    #[arg(long = "member-correspondence", required = true)]
    member_correspondence: Vec<String>,
}

#[derive(Args)]
struct ThreadValidateArgs {
    #[arg(long)]
    before_thread: String,
    #[arg(long)]
    before_impact: String,
    #[arg(long)]
    after_thread: String,
    #[arg(long)]
    after_impact: String,
    /// Total before=after mapping for the selected provider/consumer pair.
    #[arg(long = "member-correspondence", required = true)]
    member_correspondence: Vec<String>,
    /// Closed, inert codeclew-kotlin-change-coverage-document/1.0 JSON file.
    #[arg(long)]
    coverage: PathBuf,
}

#[derive(Args)]
struct ThreadIdArgs {
    #[arg(long)]
    thread: String,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OutputMode {
    Json,
    HumanCapabilities,
    HumanDoctor,
    HumanUpgrade,
}

impl OutputMode {
    fn from_cli(cli: &Cli) -> Self {
        match &cli.command {
            Command::Capabilities(args) if args.human => Self::HumanCapabilities,
            Command::Doctor(args) if args.human => Self::HumanDoctor,
            Command::Upgrade => Self::HumanUpgrade,
            _ => Self::Json,
        }
    }

    fn command_name(self) -> &'static str {
        match self {
            Self::Json => "request",
            Self::HumanCapabilities => "capabilities",
            Self::HumanDoctor => "doctor",
            Self::HumanUpgrade => "upgrade",
        }
    }
}

fn main() -> ExitCode {
    let started = std::time::Instant::now();
    let cli = Cli::parse();
    let output_mode = OutputMode::from_cli(&cli);
    let result = run(cli);
    if output_mode == OutputMode::Json {
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
    }
    match result {
        Ok(value) => {
            let rendered = match output_mode {
                OutputMode::Json => canonical::compact(&value).unwrap_or_else(|_| "{}".into()),
                OutputMode::HumanCapabilities => human_capabilities(&value),
                OutputMode::HumanDoctor => human_doctor(&value),
                OutputMode::HumanUpgrade => unreachable!("upgrade cannot succeed in source mode"),
            };
            println!("{rendered}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            let code = exit_code(&error.code);
            let rendered = if output_mode == OutputMode::Json {
                canonical::compact(&json!({"schema":"codeclew-error/2.0","error":error}))
                    .unwrap_or_else(|_| "{}".into())
            } else {
                human_error(output_mode, &error)
            };
            println!("{rendered}");
            ExitCode::from(code)
        }
    }
}

fn human_capabilities(value: &Value) -> String {
    let status = value["status"].as_str().unwrap_or("UNKNOWN");
    let runtime_mode = value["runtimeMode"].as_str().unwrap_or("UNKNOWN");
    let matrix = &value["supportMatrix"];
    let platforms = matrix["operatingSystems"]
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(platform_label)
                .collect::<Vec<_>>()
                .join(", ")
        })
        .unwrap_or_else(|| "unknown".into());

    let mut report = String::new();
    let _ = writeln!(report, "Codeclew capabilities");
    let _ = writeln!(report, "Status: {}", status.replace('_', " "));
    let _ = writeln!(report, "Runtime: {runtime_mode}");
    let _ = writeln!(report, "Platforms: {platforms}");
    let _ = writeln!(report, "\nLanguage profiles:");

    if let Some(profiles) = matrix["profiles"].as_array() {
        for profile in profiles {
            let language = profile["language"]
                .as_str()
                .map(language_label)
                .unwrap_or("Unknown");
            let version = profile["compilerVersion"]
                .as_str()
                .or_else(|| profile["engineVersion"].as_str())
                .unwrap_or("unspecified version");
            let build = profile["buildSystem"].as_str().map(build_system_label);
            let access = if profile["mutation"].as_bool() == Some(true) {
                "read and change"
            } else {
                "read only"
            };
            let profile_status = profile["status"]
                .as_str()
                .unwrap_or("UNKNOWN")
                .replace('_', " ")
                .to_ascii_lowercase();
            let build_suffix = build.map(|name| format!(" / {name}")).unwrap_or_default();
            let _ = writeln!(
                report,
                "  - {language} {version}{build_suffix}: {access} ({profile_status})"
            );
        }
    } else {
        let _ = writeln!(report, "  - No profile information available");
    }

    let minimum = matrix["threads"]["minimumMembers"].as_u64().unwrap_or(0);
    let maximum = matrix["threads"]["maximumMembers"].as_u64().unwrap_or(0);
    let thread_status = matrix["threads"]["status"]
        .as_str()
        .unwrap_or("UNKNOWN")
        .replace('_', " ")
        .to_ascii_lowercase();
    let _ = writeln!(
        report,
        "\nMulti-repository threads: {thread_status}, {minimum}-{maximum} members"
    );

    let _ = writeln!(report, "\nPackaged workers:");
    if let Some(workers) = value["packagedWorkers"].as_array() {
        if workers.is_empty() {
            let _ = writeln!(report, "  - None");
        }
        for worker in workers {
            let runtime_name = worker["runtimeName"].as_str().unwrap_or("unknown");
            let compiler = worker["compilerVersion"].as_str().unwrap_or("unknown");
            let _ = writeln!(report, "  - {runtime_name}: Kotlin {compiler}");
        }
    } else {
        let _ = writeln!(report, "  - Unavailable");
    }

    let _ = write!(
        report,
        "\nPrivacy: this report contains no source, repository identity, or absolute paths.\n\
         Run without --human for canonical JSON."
    );
    report
}

fn human_doctor(value: &Value) -> String {
    let status = value["status"].as_str().unwrap_or("UNKNOWN");
    let runtime_mode = value["runtimeMode"].as_str().unwrap_or("UNKNOWN");
    let mut report = String::new();
    let _ = writeln!(report, "Codeclew doctor");
    let _ = writeln!(
        report,
        "Status: {}",
        if status == "PASS" {
            "READY"
        } else {
            "ACTION REQUIRED"
        }
    );
    let _ = writeln!(report, "Runtime: {runtime_mode}");
    let _ = writeln!(report, "\nChecks:");

    if let Some(checks) = value["checks"].as_array() {
        for check in checks {
            let id = check["checkId"].as_str().unwrap_or("unknown");
            let check_status = check["status"].as_str().unwrap_or("UNKNOWN");
            let requirement = if check["required"].as_bool() == Some(true) {
                "required"
            } else {
                "optional"
            };
            let _ = writeln!(
                report,
                "  [{check_status}] {} ({requirement})",
                doctor_check_label(id)
            );
            if check_status != "PASS"
                && let Some(remediation) = check["remediationId"].as_str()
            {
                let _ = writeln!(report, "    Next: {}", remediation_label(remediation));
            }
        }
    } else {
        let _ = writeln!(report, "  No check information available");
    }

    let _ = write!(
        report,
        "\nPrivacy: this report contains no source, repository identity, or absolute paths.\n\
         Run without --human for canonical JSON."
    );
    report
}

fn human_error(output_mode: OutputMode, error: &ClewError) -> String {
    format!(
        "Codeclew {}\nStatus: FAILED\nError: {:?}\n{}\nRetryable: {}",
        output_mode.command_name(),
        error.code,
        error.message,
        if error.retryable { "yes" } else { "no" }
    )
}

fn platform_label(value: &str) -> &str {
    match value {
        "linux" => "Linux",
        "macos" => "macOS",
        other => other,
    }
}

fn language_label(value: &str) -> &str {
    match value {
        "java" => "Java",
        "kotlin" => "Kotlin",
        "python" => "Python",
        "rust" => "Rust",
        other => other,
    }
}

fn build_system_label(value: &str) -> &str {
    match value {
        "GRADLE_WRAPPER" => "Gradle wrapper",
        "MAVEN" => "Maven",
        other => other,
    }
}

fn doctor_check_label(id: &str) -> &str {
    match id {
        "platform.posix" => "Supported POSIX platform",
        "tool.git" => "Git is available",
        "tool.python3" => "Python 3.11+ is available",
        "tool.java" => "JDK 21 is available",
        "tool.rustc" => "Rust compiler is available",
        "tool.cargo" => "Cargo is available",
        "state.free-space" => "At least 6 GiB is free in Codeclew state",
        "runtime.kotlin24" => "Qualified Kotlin 2.4 runtime is installed",
        "runtime.kotlin23" => "Kotlin 2.3 preview runtime is installed",
        "repository.available" => "Target repository is available",
        "repository.git" => "Target is a Git repository",
        "repository.clean" => "Target worktree is clean",
        "repository.target-ref-at-head" => "Target ref points to HEAD",
        other => other,
    }
}

fn remediation_label(id: &str) -> &str {
    match id {
        "USE_SUPPORTED_POSIX_HOST" => "use a supported macOS or Linux host",
        "INSTALL_GIT" => "install Git and make it available in PATH",
        "INSTALL_PYTHON_3_11" => "install Python 3.11 or newer",
        "INSTALL_JDK_21" => "install JDK 21 and make java available in PATH",
        "INSTALL_RUST_1_92" => "install the pinned Rust 1.92 toolchain",
        "FREE_6_GIB_ON_STATE_VOLUME" => "free at least 6 GiB on the state volume",
        "INSTALL_QUALIFIED_RUNTIME" => "install or rebuild the qualified runtime",
        "INSTALL_KOTLIN23_PREVIEW_COMPONENT" => "install the optional Kotlin 2.3 preview component",
        "SELECT_EXISTING_REPOSITORY" => "select an existing repository",
        "SELECT_GIT_REPOSITORY" => "select a valid Git repository",
        "CLEAN_TARGET_WORKTREE" => "commit, stash, or use a separate clean worktree",
        "CHECKOUT_TARGET_REF_AT_HEAD" => "check out the target ref at HEAD",
        other => other,
    }
}

fn run(cli: Cli) -> Result<Value, ClewError> {
    match cli.command {
        Command::Capabilities(_) => capabilities(&active_runtime()?),
        Command::Doctor(args) => doctor(
            &active_runtime()?,
            args.repo.as_deref(),
            args.target_ref.as_deref(),
        ),
        Command::Upgrade => Err(ClewError::new(
            ErrorCode::InvalidInput,
            "this is a source checkout; update it through the approved Git commit or tag",
        )),
        Command::Change {
            command: ChangeCommand::Open(args),
        } => change_open(args),
        Command::Change {
            command: ChangeCommand::CheckFreshness(args),
        } => {
            let (session, _) = SessionAuthority::load(&args.session)?;
            serde_json::to_value(session.freshness()?).map_err(internal)
        }
        Command::Change {
            command: ChangeCommand::Prepare(args),
        } => change_prepare(args),
        Command::Change {
            command: ChangeCommand::Status(args),
        } => with_schema("codeclew-change-status/1.0", task_run_status(&args.run)?),
        Command::Change {
            command: ChangeCommand::Publish(args),
        } => with_schema(
            "codeclew-change-publish/1.0",
            publish_task_run(
                &args.session,
                &args.run,
                args.allow_conditional,
                args.prepared_authority_digest.as_deref(),
                &args.acknowledge_obligations,
            )?,
        ),
        Command::Change {
            command: ChangeCommand::Recover(args),
        } => with_schema(
            "codeclew-change-recover/1.0",
            recover_task_run(&args.session, &args.run)?,
        ),
        Command::Mission {
            command: MissionCommand::Open(args),
        } => {
            let source = read_private_diagnostic_input(&args.spec, mission::MAX_CHANGE_SPEC_BYTES)?;
            serde_json::to_value(mission::open(&args.sessions, &source)?).map_err(internal)
        }
        Command::Mission {
            command: MissionCommand::Record(args),
        } => serde_json::to_value(mission::record(
            &args.mission,
            &args.session,
            args.context.as_deref(),
            args.plan.as_deref(),
            args.run.as_deref(),
        )?)
        .map_err(internal),
        Command::Mission {
            command: MissionCommand::Develop(args),
        } => {
            let source = read_private_diagnostic_input(
                &args.record,
                mission::development_record::MAX_INPUT_BYTES,
            )?;
            mission::development_record::create(&args.mission, &source)
        }
        Command::Mission {
            command: MissionCommand::Dossier(args),
        } => {
            let format = match args.format {
                DossierFormatArg::Json => mission::development_record::RenderFormat::Json,
                DossierFormatArg::Markdown => mission::development_record::RenderFormat::Markdown,
                DossierFormatArg::Dot => mission::development_record::RenderFormat::Dot,
            };
            mission::development_record::render(
                &args.mission,
                &args.record,
                format,
                args.node.as_deref(),
            )
        }
        Command::Mission {
            command: MissionCommand::Inspect(args),
        } => serde_json::to_value(mission::inspect(&args.mission)?).map_err(internal),
        Command::Mission {
            command: MissionCommand::Status(args),
        } => serde_json::to_value(mission::status(&args.mission)?).map_err(internal),
        Command::Mission {
            command: MissionCommand::Close(args),
        } => serde_json::to_value(mission::close(&args.mission)?).map_err(internal),
        Command::Workspace {
            command: WorkspaceCommand::Open(args),
        } => {
            let source = read_private_diagnostic_input(
                &args.catalog,
                workspace::MAX_WORKSPACE_CATALOG_BYTES,
            )?;
            serde_json::to_value(workspace::open(&source)?).map_err(internal)
        }
        Command::Workspace {
            command: WorkspaceCommand::Inspect(args),
        } => serde_json::to_value(workspace::inspect(&args.workspace)?).map_err(internal),
        Command::Workspace {
            command: WorkspaceCommand::Context(args),
        } => workspace::context(&args.workspace, &args.intent, &args.terms, args.max_roots),
        Command::Workspace {
            command: WorkspaceCommand::Prepare(args),
        } => {
            let source = read_private_diagnostic_input(
                &args.request,
                clew::workspace_prepare::MAX_WORKSPACE_PREPARE_INPUT_BYTES,
            )?;
            prepare_workspace(&args.workspace, &source)
        }
        Command::Workspace {
            command: WorkspaceCommand::Observe(args),
        } => {
            let request = read_private_diagnostic_input(
                &args.request,
                clew::scenario_receipt::MAX_SCENARIO_INPUT_BYTES,
            )?;
            let evidence = read_private_diagnostic_input(
                &args.evidence,
                clew::scenario_receipt::MAX_SCENARIO_RAW_EVIDENCE_BYTES,
            )?;
            serde_json::to_value(clew::scenario_receipt::record(
                &args.workspace,
                &request,
                &evidence,
            )?)
            .map_err(internal)
        }
        Command::Workspace {
            command: WorkspaceCommand::Publish(args),
        } => {
            let request = read_private_diagnostic_input(
                &args.request,
                clew::workspace_publish::MAX_WORKSPACE_PUBLISH_INPUT_BYTES,
            )?;
            publish_workspace(&args.workspace, Some(&request), None)
        }
        Command::Workspace {
            command: WorkspaceCommand::Recover(args),
        } => publish_workspace(&args.workspace, None, Some(&args.publication)),
        Command::Workspace {
            command: WorkspaceCommand::Close(args),
        } => serde_json::to_value(workspace::close(&args.workspace)?).map_err(internal),
        Command::Session {
            command: SessionCommand::Open(args),
        } => {
            let session = open_session(&args)?;
            Ok(json!({"schema":"codeclew-session-open/4.0","status":"OPEN","session":session}))
        }
        Command::Session {
            command: SessionCommand::Inspect(args),
        } => {
            let (session, _) = SessionAuthority::load(&args.session)?;
            let lifecycle = session.lifecycle()?;
            Ok(
                json!({"schema":"codeclew-session-inspect/4.0","session":session,"lifecycle":lifecycle}),
            )
        }
        Command::Session {
            command: SessionCommand::Close(args),
        } => {
            let (session, _) = SessionAuthority::load_for_cleanup(&args.session)?;
            let lifecycle = session.close()?;
            Ok(json!({"schema":"codeclew-session-lifecycle-result/1.0","lifecycle":lifecycle}))
        }
        Command::Session {
            command: SessionCommand::Abort(args),
        } => {
            let (session, _) = SessionAuthority::load(&args.session)?;
            let lifecycle = session.abort()?;
            Ok(json!({"schema":"codeclew-session-lifecycle-result/1.0","lifecycle":lifecycle}))
        }
        Command::Session {
            command: SessionCommand::Relocate(args),
        } => {
            let (session, _) = SessionAuthority::load(&args.session)?;
            let lifecycle = session.relocate(&absolute(&args.repo)?)?;
            Ok(
                json!({"schema":"codeclew-session-relocate-result/1.0","sessionId":session.session_id,
                "repositoryKey":session.repository_key,"lifecycle":lifecycle}),
            )
        }
        Command::Session {
            command: SessionCommand::Gc(args),
        } => {
            let (session, _) = SessionAuthority::load_for_cleanup(&args.session)?;
            let lifecycle = session.gc(args.force)?;
            Ok(json!({"schema":"codeclew-session-gc-result/1.0","lifecycle":lifecycle}))
        }
        Command::Context {
            command: ContextCommand::Create(args),
        } => {
            let (session, _) = SessionAuthority::load(&args.session)?;
            create_context(&session, args.intent, args.terms, args.max_roots)
        }
        Command::Context {
            command: ContextCommand::Expand(args),
        } => {
            let (session, _) = SessionAuthority::load(&args.session)?;
            session.require_open()?;
            let parent = session.load_context(&args.context)?;
            let additional_terms = args.terms;
            let mut terms = parent.terms.clone();
            terms.extend(additional_terms.iter().cloned());
            terms.sort();
            terms.dedup();
            let intent = args.intent.unwrap_or_else(|| parent.intent.clone());
            validate_context_request(&intent, &terms)?;
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
        Command::Thread {
            command: ThreadCommand::Open(args),
        } => {
            let thread = ThreadAuthority::open(parse_thread_members(args)?)?;
            Ok(json!({
                "schema":"codeclew-thread-open/1.0",
                "status":"OPEN",
                "thread":thread,
            }))
        }
        Command::Thread {
            command: ThreadCommand::Context(args),
        } => {
            let (thread, _) = ThreadAuthority::load(&args.thread)?;
            let context =
                create_thread_context(&thread, &args.intent, &args.terms, args.max_roots)?;
            bounded_thread_context_stdout(&context)
        }
        Command::Thread {
            command: ThreadCommand::Callables(args),
        } => {
            let (thread, _) = ThreadAuthority::load(&args.thread)?;
            clew::thread_callables_service::create_bounded(
                &thread,
                &args.context,
                clew::thread_callables_service::ThreadCallablesRequest {
                    task_id: args.task_id,
                    pair_id: args.pair_id,
                    provider_member: args.provider,
                    consumer_member: args.consumer,
                    terms: args.terms,
                },
            )
        }
        Command::Thread {
            command: ThreadCommand::Flow(args),
        } => {
            let root_kind = match args.root_kind {
                ThreadFlowRootKindArg::FullSymbol => clew::thread_flow::FlowRootKind::FullSymbol,
            };
            let direction = match args.direction {
                ThreadFlowDirectionArg::Downstream => clew::thread_flow::FlowDirection::Downstream,
            };
            let (thread, _) = ThreadAuthority::load(&args.thread)?;
            let root = clew::thread_flow_service::create(
                &thread,
                &args.fact_set,
                clew::thread_flow_service::ThreadFlowServiceRequest {
                    pair_id: args.pair_id,
                    member_alias: args.member,
                    root_kind,
                    root: args.root,
                    direction,
                    max_depth: args.max_depth,
                },
            )?;
            clew::thread_flow_service::inspect(&thread, &root.flow_id)
        }
        Command::Thread {
            command: ThreadCommand::Explain(args),
        } => {
            let bytes = read_bounded_regular_file(
                &args.claims,
                clew::explanation::MAX_CLAIM_DOCUMENT_BYTES,
                "claim document is missing, unsafe, or exceeds 1 MiB",
            )?;
            let document = clew::explanation::parse_claim_document(&bytes)?;
            let (thread, _) = ThreadAuthority::load(&args.thread)?;
            let root = clew::explanation_service::create(&thread, &args.flow, document)?;
            clew::explanation_service::bounded_stdout(&root)
        }
        Command::Thread {
            command: ThreadCommand::Render(args),
        } => {
            let detail = match args.detail {
                ExplanationDetailArg::Summary => clew::explanation_render::DetailLevel::Summary,
                ExplanationDetailArg::Scenario => clew::explanation_render::DetailLevel::Scenario,
                ExplanationDetailArg::Technical => clew::explanation_render::DetailLevel::Technical,
                ExplanationDetailArg::Evidence => clew::explanation_render::DetailLevel::Evidence,
                ExplanationDetailArg::Compiler => clew::explanation_render::DetailLevel::Compiler,
            };
            let format = match args.format {
                ExplanationFormatArg::Json => clew::explanation_render::RenderFormat::Json,
                ExplanationFormatArg::Markdown => clew::explanation_render::RenderFormat::Markdown,
            };
            let (thread, _) = ThreadAuthority::load(&args.thread)?;
            clew::explanation_service::render(&thread, &args.explanation, detail, format)
        }
        Command::Thread {
            command: ThreadCommand::ExplanationStatus(args),
        } => {
            let member_correspondence = args
                .member_correspondence
                .iter()
                .map(|value| {
                    let (before, after) = parse_binding(value, "member correspondence")?;
                    Ok(clew::thread_change_set::MemberCorrespondence {
                        before_member_alias: before.into(),
                        after_member_alias: after.into(),
                    })
                })
                .collect::<Result<Vec<_>, ClewError>>()?;
            let (old_thread, _) = ThreadAuthority::load(&args.thread)?;
            let (against_thread, _) = ThreadAuthority::load(&args.against_thread)?;
            let root = clew::explanation_freshness_service::create(
                &old_thread,
                &against_thread,
                clew::explanation_freshness_service::ExplanationFreshnessServiceRequest {
                    old_explanation_id: args.explanation,
                    against_fact_set_id: args.against_fact_set,
                    against_flow_id: args.against_flow,
                    member_correspondence,
                },
            )?;
            clew::explanation_freshness_service::bounded_stdout(&root)
        }
        Command::Thread {
            command: ThreadCommand::Impact(args),
        } => {
            let subject = match args.subject_kind {
                ThreadImpactSubjectKindArg::FullSymbol => {
                    if args.declarations_only || args.declaration_name_only {
                        return Err(ClewError::new(
                            ErrorCode::InvalidInput,
                            "thread impact declaration filters are accepted only for token subjects",
                        ));
                    }
                    clew::thread_impact::KotlinImpactSubject::FullSymbol {
                        symbol_identity: args.subject,
                        member_alias: args.member,
                    }
                }
                ThreadImpactSubjectKindArg::CallableFamily => {
                    if args.declarations_only || args.declaration_name_only {
                        return Err(ClewError::new(
                            ErrorCode::InvalidInput,
                            "thread impact declaration filters are accepted only for token subjects",
                        ));
                    }
                    if args.member.is_some() {
                        return Err(ClewError::new(
                            ErrorCode::InvalidInput,
                            "thread impact --member is not accepted for callable-family subjects",
                        ));
                    }
                    clew::thread_impact::KotlinImpactSubject::CallableFamily {
                        callable_id: args.subject,
                    }
                }
                ThreadImpactSubjectKindArg::Token => {
                    clew::thread_impact::KotlinImpactSubject::Token {
                        term: args.subject,
                        member_alias: args.member,
                        declarations_only: args.declarations_only,
                        declaration_name_only: args.declaration_name_only,
                    }
                }
            };
            let (thread, _) = ThreadAuthority::load(&args.thread)?;
            let root = clew::thread_impact_service::create(
                &thread,
                &args.fact_set,
                clew::thread_impact_service::ThreadImpactServiceRequest {
                    pair_id: args.pair_id,
                    subject,
                },
            )?;
            clew::thread_impact_service::bounded_stdout(&root)
        }
        Command::Thread {
            command: ThreadCommand::Validate(args),
        } => {
            let coverage_document = read_bounded_regular_file(
                &args.coverage,
                clew::thread_change_set::MAX_CHANGE_COVERAGE_DOCUMENT_BYTES,
                "coverage document is missing, unsafe, or exceeds 2 MiB",
            )?;
            let member_correspondence = args
                .member_correspondence
                .iter()
                .map(|value| {
                    let (before, after) = parse_binding(value, "member correspondence")?;
                    Ok(clew::thread_change_set::MemberCorrespondence {
                        before_member_alias: before.into(),
                        after_member_alias: after.into(),
                    })
                })
                .collect::<Result<Vec<_>, ClewError>>()?;
            let (before, _) = ThreadAuthority::load(&args.before_thread)?;
            let (after, _) = ThreadAuthority::load(&args.after_thread)?;
            let root = clew::thread_change_set_service::create(
                &before,
                &after,
                clew::thread_change_set_service::ThreadChangeSetServiceRequest {
                    before_impact_id: args.before_impact,
                    after_impact_id: args.after_impact,
                    member_correspondence,
                    coverage_document,
                },
            )?;
            clew::thread_change_set_service::bounded_stdout(&root)
        }
        Command::Thread {
            command: ThreadCommand::Close(args),
        } => {
            let (thread, _) = ThreadAuthority::load(&args.thread)?;
            let lifecycle = thread.close()?;
            Ok(json!({
                "schema":"codeclew-thread-lifecycle-result/1.0",
                "threadId":thread.thread_id,
                "lifecycle":lifecycle,
            }))
        }
        Command::Thread {
            command: ThreadCommand::Gc(args),
        } => {
            let (thread, _) = ThreadAuthority::load(&args.thread)?;
            let lifecycle = thread.gc()?;
            Ok(json!({
                "schema":"codeclew-thread-gc-result/1.0",
                "threadId":thread.thread_id,
                "lifecycle":lifecycle,
            }))
        }
        Command::Plan {
            command: PlanCommand::Validate(args),
        } => {
            reject_thread_context_id(&args.context)?;
            reject_thread_session_id(&args.session)?;
            let (session, _) = SessionAuthority::load(&args.session)?;
            session.require_open()?;
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
        } => {
            reject_thread_context_id(&args.context)?;
            reject_thread_session_id(&args.session)?;
            start_task_run(&args.session, &args.context, &args.plan)
        }
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
        } => publish_task_run(
            &args.session,
            &args.run,
            args.allow_conditional,
            args.prepared_authority_digest.as_deref(),
            &args.acknowledge_obligations,
        ),
        Command::Session {
            command: SessionCommand::Recover(args),
        } => recover_task_run(&args.session, &args.run),
        Command::Support {
            command: SupportCommand::Summarize(args),
        } => {
            let bytes = read_private_diagnostic_input(&args.input, 1024 * 1024)?;
            let value: Value = serde_json::from_slice(&bytes)
                .map_err(|_| invalid("diagnostic input is not one JSON object"))?;
            support_summary(&value)
        }
        Command::InternalTaskRunExecute(args) => execute_task_run(&args.run),
    }
}

fn active_runtime() -> Result<RuntimeAuthority, ClewError> {
    RuntimeAuthority::from_environment()?.ok_or_else(|| {
        ClewError::new(
            ErrorCode::WorkerPreparationRequired,
            "operational commands must be launched through ./clew",
        )
    })
}

fn open_session(args: &SessionOpenArgs) -> Result<SessionAuthority, ClewError> {
    let policy = match args.model_cache {
        ModelCachePolicyArg::NonCacheable => ModelCachePolicy::NonCacheable,
        ModelCachePolicyArg::TrackedManifest => ModelCachePolicy::TrackedManifest,
        ModelCachePolicyArg::SealedExternal => ModelCachePolicy::SealedExternal,
    };
    SessionAuthority::open(
        &absolute(&args.repo)?,
        &args.target_ref,
        match args.language {
            SessionLanguageArg::Java => SessionLanguage::Java,
            SessionLanguageArg::Kotlin => SessionLanguage::Kotlin,
            SessionLanguageArg::Python => SessionLanguage::Python,
            SessionLanguageArg::Rust => SessionLanguage::Rust,
        },
        &args.compilation,
        args.generation_jobs,
        policy,
        args.external_build_state.as_deref(),
    )
}

fn create_context(
    session: &SessionAuthority,
    intent: String,
    terms: Vec<String>,
    max_roots: usize,
) -> Result<Value, ClewError> {
    validate_context_request(&intent, &terms)?;
    session.require_open()?;
    let (projection, evidence) =
        clew::context_v2::create(session, &intent, &terms, max_roots, None)?;
    bounded_context_stdout(&session.store_context(None, intent, terms, projection, evidence)?)
}

fn change_open(args: ChangeOpenArgs) -> Result<Value, ClewError> {
    let session = open_session(&args.session)?;
    match create_context(&session, args.intent, args.terms, args.max_roots) {
        Ok(context) => Ok(json!({
            "schema":"codeclew-change-open/1.0",
            "status":"OPEN",
            "session":session,
            "context":context,
        })),
        Err(error) => Err(change_open_failure(error, &session.session_id, || {
            session.abort()?;
            session.gc(false)?;
            Ok(())
        })),
    }
}

fn change_open_failure(
    original: ClewError,
    session_id: &str,
    cleanup: impl FnOnce() -> Result<(), ClewError>,
) -> ClewError {
    match cleanup() {
        Ok(()) => original,
        Err(cleanup_error) => ClewError::new(
            ErrorCode::TransactionRecoveryRequired,
            format!(
                "change open failed and session cleanup requires recovery: {:?}",
                cleanup_error.code
            ),
        )
        .with_transaction(session_id),
    }
}

fn change_prepare(args: ChangePrepareArgs) -> Result<Value, ClewError> {
    reject_thread_context_id(&args.context)?;
    reject_thread_session_id(&args.session)?;
    let (session, _) = SessionAuthority::load(&args.session)?;
    session.require_open()?;
    let metadata = std::fs::symlink_metadata(&args.plan).map_err(io_error)?;
    if metadata.file_type().is_symlink()
        || !metadata.is_file()
        || metadata.len() as usize > clew::session::MAX_PLAN_BYTES
    {
        return Err(invalid("plan is missing, unsafe, or exceeds 1 MiB"));
    }
    let plan =
        session.validate_plan(&args.context, &std::fs::read(&args.plan).map_err(io_error)?)?;
    let status = start_task_run(&args.session, &args.context, &plan.plan_id)?;
    Ok(json!({
        "schema":"codeclew-change-prepare/1.0",
        "status":"STARTED",
        "sessionId":args.session,
        "contextId":args.context,
        "planId":plan.plan_id,
        "run":status.get("run").cloned().unwrap_or(Value::Null),
        "candidate":status.get("candidate").cloned().unwrap_or(Value::Null),
    }))
}

fn with_schema(schema: &str, mut value: Value) -> Result<Value, ClewError> {
    let object = value
        .as_object_mut()
        .ok_or_else(|| internal("command result is not an object"))?;
    object.insert("schema".into(), Value::String(schema.into()));
    Ok(value)
}

fn start_task_run(session_id: &str, context_id: &str, plan_id: &str) -> Result<Value, ClewError> {
    reject_thread_coverage_id(plan_id)?;
    reject_thread_context_id(context_id)?;
    reject_thread_session_id(session_id)?;
    let (session, _) = SessionAuthority::load(session_id)?;
    require_mutation_request(&session, context_id, plan_id)?;
    let record = RunRecord::created(&session, context_id, plan_id)?;
    if !record.create_once()? {
        return task_run_status(&record.run_id);
    }
    spawn_task_run(&record.run_id)?;
    task_run_status(&record.run_id)
}

fn prepare_workspace(workspace_id: &str, source: &[u8]) -> Result<Value, ClewError> {
    let authority = clew::workspace_prepare::resolve(workspace_id, source)?;
    if let Some(after) = clew::workspace_prepare::retained_after(&authority)? {
        return bounded_after_workspace(after);
    }
    for member in &authority.members {
        start_task_run(&member.session_id, &member.context_id, &member.plan_id)?;
    }
    loop {
        match clew::workspace_prepare::observe_and_finalize(&authority)? {
            clew::workspace_prepare::WorkspacePrepareObservation::Preparing(_) => {
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
            clew::workspace_prepare::WorkspacePrepareObservation::Prepared(after) => {
                return bounded_after_workspace(after);
            }
        }
    }
}

fn bounded_after_workspace(
    after: clew::workspace_prepare::AfterWorkspace,
) -> Result<Value, ClewError> {
    let value = serde_json::to_value(after).map_err(internal)?;
    if canonical::bytes(&value).map_err(internal)?.len() > 64 * 1024 {
        return Err(ClewError::new(
            ErrorCode::SliceBudgetExceeded,
            "AfterWorkspace stdout exceeds 64 KiB",
        ));
    }
    Ok(value)
}

fn publish_workspace(
    workspace_id: &str,
    request: Option<&[u8]>,
    publication_id: Option<&str>,
) -> Result<Value, ClewError> {
    let mut projection = match (request, publication_id) {
        (Some(source), None) => clew::workspace_publish::resolve(workspace_id, source)?,
        (None, Some(publication_id)) => {
            clew::workspace_publish::load(workspace_id, publication_id)?.1
        }
        _ => return Err(invalid("workspace publication authority is ambiguous")),
    };
    for _ in 0..16 {
        if projection.status == clew::workspace_publish::WorkspacePublicationStatus::PublishedAll {
            return bounded_workspace_publication(projection);
        }
        let (authority, latest) =
            clew::workspace_publish::load(workspace_id, &projection.publication_id)?;
        projection = latest;
        if projection.status != clew::workspace_publish::WorkspacePublicationStatus::Publishing {
            projection = clew::workspace_publish::begin_next(
                workspace_id,
                &authority.publication_id,
                &projection.ledger_head,
            )?;
        }
        let member = clew::workspace_publish::active_member(&authority, &projection)?
            .ok_or_else(|| internal("publication has no active member"))?
            .clone();
        let record = RunRecord::load(&member.run_id)?;
        let result = if matches!(
            record.status,
            RunStatus::Publishing | RunStatus::WorktreeRecoveryRequired
        ) {
            recover_task_run(&member.session_id, &member.run_id)
        } else {
            publish_task_run(
                &member.session_id,
                &member.run_id,
                member.allow_conditional,
                member
                    .allow_conditional
                    .then_some(member.prepared_authority_digest.as_str()),
                &member.acknowledge_obligations,
            )
        };
        if let Err(error) = result {
            projection = clew::workspace_publish::record_failure(
                workspace_id,
                &authority.publication_id,
                &projection.ledger_head,
                &member.alias,
                error.code,
            )?;
            return bounded_workspace_publication(projection);
        }
        let published = RunRecord::load(&member.run_id)?;
        if matches!(
            published.status,
            RunStatus::Published | RunStatus::PublishedConditional
        ) && published.final_commit.as_deref() == Some(member.candidate_oid.as_str())
        {
            projection = clew::workspace_publish::record_published(
                workspace_id,
                &authority.publication_id,
                &projection.ledger_head,
                &member.alias,
                &member.candidate_oid,
            )?;
        }
    }
    Err(ClewError::new(
        ErrorCode::ResourceLimit,
        "workspace publication exceeded its bounded recovery transitions",
    ))
}

fn bounded_workspace_publication(
    projection: clew::workspace_publish::WorkspacePublicationProjection,
) -> Result<Value, ClewError> {
    let value = serde_json::to_value(projection).map_err(internal)?;
    if canonical::bytes(&value).map_err(internal)?.len() > 64 * 1024 {
        return Err(ClewError::new(
            ErrorCode::SliceBudgetExceeded,
            "workspace publication stdout exceeds 64 KiB",
        ));
    }
    Ok(value)
}

fn spawn_task_run(run_id: &str) -> Result<(), ClewError> {
    #[cfg(unix)]
    use std::os::unix::process::CommandExt;
    let state = clew::state::StateAuthority::process_default()?;
    let root = state.run_root(run_id)?;
    let stdout = state.open_private_append(&root.join("stdout.log"))?;
    let stderr = state.open_private_append(&root.join("stderr.log"))?;
    let mut command = std::process::Command::new(std::env::current_exe().map_err(io_error)?);
    command
        .args(["__task-run-execute", "--run", run_id])
        .stdin(Stdio::null())
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr));
    #[cfg(unix)]
    command.process_group(0);
    // The child owns the CREATED -> PREPARING transition and records its own
    // process identity in the same ledger event. A parent-side PID update would
    // race that transition and turn one deterministic run into two stale CAS
    // writers.
    command.spawn().map_err(io_error)?;
    Ok(())
}

fn task_run_status(run_id: &str) -> Result<Value, ClewError> {
    reject_thread_coverage_id(run_id)?;
    let run = RunRecord::load(run_id)?;
    let state = clew::state::StateAuthority::process_default()?;
    let prepared_path = state.run_root(run_id)?.join("prepared-v2.json");
    let candidate = if state.private_file_exists(&prepared_path)? {
        let prepared = read_json::<clew::task_run_v2::PreparedCandidateV2>(
            &state,
            &prepared_path,
            16 * 1024 * 1024,
        )?;
        Some(clew::task_run_v2::public_candidate_status(&prepared)?)
    } else {
        None
    };
    Ok(json!({"schema":"codeclew-task-run-status/3.0","run":run,"candidate":candidate}))
}

fn resume_task_run(run_id: &str) -> Result<Value, ClewError> {
    reject_thread_coverage_id(run_id)?;
    let mut record = RunRecord::load(run_id)?;
    let (session, _) = SessionAuthority::load(&record.session_id)?;
    require_mutation_request(&session, &record.context_id, &record.plan_id)?;
    // Admission stays locked until the inactive run is either classified as
    // recovery-required or durably returned to CREATED and handed to the
    // supervisor. close/abort use the same session -> run lock order.
    let _admission = session.open_admission()?;
    if matches!(
        record.status,
        RunStatus::ReadyToPublish
            | RunStatus::ReadyToPublishConditional
            | RunStatus::ValidatedConditional
            | RunStatus::Published
            | RunStatus::PublishedConditional
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
    let (session, _) = SessionAuthority::load(&record.session_id)?;
    let candidate_root = record.candidate_root()?;
    if let Some(commit) = clew::task_run_v2::checkpoint_commit(&candidate_root)?.or(
        clew::task_run_v2::recoverable_candidate_commit(&session, &candidate_root)?,
    ) {
        record.candidate_commit = Some(commit);
        record.status = RunStatus::WorktreeRecoveryRequired;
        record.failure = Some(json!({"code":"WORKTREE_RECOVERY_REQUIRED",
            "message":"committed candidate requires deterministic preparation recovery"}));
        record.save()?;
        return task_run_status(run_id);
    }
    clew::task_run_v2::discard_precommit_candidate(&session, &candidate_root)?;
    record.status = RunStatus::Created;
    record.failure = None;
    record.process_id = None;
    record.process_start_token = None;
    record.save()?;
    spawn_task_run(run_id)?;
    task_run_status(run_id)
}

fn cancel_task_run(run_id: &str) -> Result<Value, ClewError> {
    reject_thread_coverage_id(run_id)?;
    let mut record = RunRecord::load(run_id)?;
    if record.status == RunStatus::Cancelled {
        return task_run_status(run_id);
    }
    if !cancellation_allowed(record.status) {
        return Err(ClewError::new(
            ErrorCode::PreconditionFailed,
            "only a created, preparing, or unpublished ready run can be cancelled",
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
    matches!(
        status,
        RunStatus::Created
            | RunStatus::Preparing
            | RunStatus::ReadyToPublish
            | RunStatus::ReadyToPublishConditional
    )
}

fn execute_task_run(run_id: &str) -> Result<Value, ClewError> {
    reject_thread_coverage_id(run_id)?;
    clew::worker::install_task_run_process_tree_supervisor()?;
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
            let mut latest = RunRecord::load(run_id)?;
            if latest.status != RunStatus::Cancelled {
                let candidate_root = latest.candidate_root()?;
                let checkpoint = clew::task_run_v2::checkpoint_commit(&candidate_root)?;
                latest.status = if let Some(commit) = checkpoint {
                    latest.candidate_commit = Some(commit);
                    RunStatus::WorktreeRecoveryRequired
                } else if error.code == ErrorCode::WorktreeRecoveryRequired {
                    RunStatus::WorktreeRecoveryRequired
                } else {
                    RunStatus::Failed
                };
                latest.failure = serde_json::to_value(&error).ok();
            }
            latest.process_id = None;
            latest.process_start_token = None;
            let _ = latest.save();
            Err(error)
        }
    }
}

fn prepare_task_run(record: &mut RunRecord) -> Result<Value, ClewError> {
    let (session, _) = SessionAuthority::load(&record.session_id)?;
    let context = session.load_context(&record.context_id)?;
    let plan = session.load_plan(&record.plan_id)?;
    if context.evidence.get("schema").and_then(Value::as_str)
        != Some(clew::context_v2::BOUNDED_CONTEXT_EVIDENCE_SCHEMA)
    {
        return Err(invalid(
            "task run requires the current multi-compilation bounded context",
        ));
    }
    let prepared =
        clew::task_run_v2::prepare(&session, &context, &plan, &record.candidate_root()?)?;
    let state = clew::state::StateAuthority::process_default()?;
    state.write_private_atomic(
        &state.run_root(&record.run_id)?.join("prepared-v2.json"),
        &canonical::bytes(&prepared).map_err(internal)?,
    )?;
    let mut latest = RunRecord::load(&record.run_id)?;
    latest.candidate_commit = Some(prepared.candidate_commit.clone());
    latest.candidate_snapshot = Some(prepared.candidate_snapshot.clone());
    latest.prepared_authority_digest = Some(prepared.prepared_authority_digest.clone());
    latest.publication_blocked = prepared.publication_blocked;
    latest.status = if latest.status == RunStatus::Cancelled {
        RunStatus::Cancelled
    } else if prepared.conditional_publish_eligible {
        RunStatus::ReadyToPublishConditional
    } else if prepared.publication_blocked {
        RunStatus::ValidatedConditional
    } else {
        RunStatus::ReadyToPublish
    };
    latest.process_id = None;
    latest.process_start_token = None;
    latest.save()?;
    *record = latest;
    Ok(json!({"schema":"codeclew-task-run-preparation/2.0","run":record,"candidate":prepared}))
}

fn publish_task_run(
    session_id: &str,
    run_id: &str,
    allow_conditional: bool,
    prepared_authority_digest: Option<&str>,
    acknowledged: &[String],
) -> Result<Value, ClewError> {
    reject_thread_coverage_id(run_id)?;
    reject_thread_session_id(session_id)?;
    let (session, _) = SessionAuthority::load(session_id)?;
    session.require_open()?;
    let mut record = RunRecord::load(run_id)?;
    require_run_session(&record, session_id)?;
    require_mutation_request(&session, &record.context_id, &record.plan_id)?;
    let state = clew::state::StateAuthority::process_default()?;
    let prepared: clew::task_run_v2::PreparedCandidateV2 = read_json(
        &state,
        &state.run_root(run_id)?.join("prepared-v2.json"),
        16 * 1024 * 1024,
    )?;
    let requested_approval = if allow_conditional {
        if prepared_authority_digest != Some(prepared.prepared_authority_digest.as_str()) {
            return Err(ClewError::new(
                ErrorCode::PreconditionFailed,
                "conditional publish digest differs from the reviewed prepared authority",
            ));
        }
        Some(clew::task_run_v2::conditional_approval(
            &session,
            &prepared,
            &record.run_id,
            &record.request_digest,
            acknowledged,
        )?)
    } else {
        if !acknowledged.is_empty() || prepared_authority_digest.is_some() {
            return Err(invalid(
                "obligation acknowledgement requires --allow-conditional",
            ));
        }
        None
    };
    require_record_prepared_authority(&record, &prepared)?;
    if record.status == RunStatus::ValidatedConditional {
        return Err(ClewError::new(
            ErrorCode::IncompleteSemanticAnalysis,
            "run is conditional but not eligible for acknowledged publication",
        ));
    }
    if record.status == RunStatus::ReadyToPublishConditional && requested_approval.is_none() {
        return Err(ClewError::new(
            ErrorCode::IncompleteSemanticAnalysis,
            "conditional run requires reviewed digest and explicit obligation acknowledgement",
        ));
    }
    if matches!(
        record.status,
        RunStatus::Published | RunStatus::PublishedConditional
    ) {
        if record.conditional_approval != requested_approval {
            return Err(ClewError::new(
                ErrorCode::PreconditionFailed,
                "published run approval differs from the retry",
            ));
        }
        return task_run_status(run_id);
    }
    if !matches!(
        (record.status, requested_approval.is_some()),
        (RunStatus::ReadyToPublish, false) | (RunStatus::ReadyToPublishConditional, true)
    ) {
        return Err(ClewError::new(
            ErrorCode::PreconditionFailed,
            "run is not ready to publish",
        ));
    }
    record.conditional_approval = requested_approval.clone();
    record.status = RunStatus::Publishing;
    record.save()?;
    match clew::task_run_v2::publish(
        &session,
        &prepared,
        &record.candidate_root()?,
        requested_approval.as_ref(),
    ) {
        Ok(publication) => {
            record.status = if requested_approval.is_some() {
                RunStatus::PublishedConditional
            } else {
                RunStatus::Published
            };
            record.final_commit = Some(prepared.candidate_commit.clone());
            record.failure = None;
            record.save()?;
            Ok(json!({"schema":"codeclew-session-publish-result/2.0",
                "run":record,"publication":publication}))
        }
        Err(error) => {
            record.status = if error.code == ErrorCode::WorktreeRecoveryRequired {
                RunStatus::WorktreeRecoveryRequired
            } else if requested_approval.is_some() {
                RunStatus::ReadyToPublishConditional
            } else {
                RunStatus::ReadyToPublish
            };
            record.failure = serde_json::to_value(&error).ok();
            record.save()?;
            Err(error)
        }
    }
}

fn require_mutation_request(
    session: &SessionAuthority,
    context_id: &str,
    plan_id: &str,
) -> Result<(), ClewError> {
    let context = session.load_context(context_id)?;
    let plan = session.load_plan(plan_id)?;
    clew::task_run_v2::require_mutation_request(session, &context, &plan)?;
    Ok(())
}

fn recover_task_run(session_id: &str, run_id: &str) -> Result<Value, ClewError> {
    reject_thread_coverage_id(run_id)?;
    reject_thread_session_id(session_id)?;
    let (session, _) = SessionAuthority::load(session_id)?;
    session.require_open()?;
    let mut record = RunRecord::load(run_id)?;
    require_run_session(&record, session_id)?;
    require_mutation_request(&session, &record.context_id, &record.plan_id)?;
    if matches!(
        record.status,
        RunStatus::Published | RunStatus::PublishedConditional
    ) {
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
    let run_root = state.run_root(run_id)?;
    let prepared_path = run_root.join("prepared-v2.json");
    let prepared_exists = state.private_file_exists(&prepared_path)?;
    if record.status == RunStatus::WorktreeRecoveryRequired
        && (!prepared_exists || record.prepared_authority_digest.is_none())
    {
        let context = session.load_context(&record.context_id)?;
        let plan = session.load_plan(&record.plan_id)?;
        match clew::task_run_v2::recover_preparation(
            &session,
            &context,
            &plan,
            &record.candidate_root()?,
        ) {
            Ok(prepared) => {
                state.write_private_atomic(
                    &prepared_path,
                    &canonical::bytes(&prepared).map_err(internal)?,
                )?;
                record.candidate_commit = Some(prepared.candidate_commit.clone());
                record.candidate_snapshot = Some(prepared.candidate_snapshot.clone());
                record.prepared_authority_digest = Some(prepared.prepared_authority_digest.clone());
                record.publication_blocked = prepared.publication_blocked;
                record.status = if prepared.conditional_publish_eligible {
                    RunStatus::ReadyToPublishConditional
                } else if prepared.publication_blocked {
                    RunStatus::ValidatedConditional
                } else {
                    RunStatus::ReadyToPublish
                };
                record.failure = None;
                record.save()?;
                return Ok(json!({"schema":"codeclew-session-recover-result/2.0",
                    "run":record,"preparation":prepared}));
            }
            Err(error) => {
                record.failure = serde_json::to_value(&error).ok();
                record.save()?;
                return Err(error);
            }
        }
    }
    if !prepared_exists {
        return Err(ClewError::new(
            ErrorCode::WorktreeRecoveryRequired,
            "publishing run lost its reviewed prepared authority",
        ));
    }
    let prepared: clew::task_run_v2::PreparedCandidateV2 =
        read_json(&state, &prepared_path, 16 * 1024 * 1024)?;
    require_record_prepared_authority(&record, &prepared)?;
    match clew::task_run_v2::recover(
        &session,
        &prepared,
        &record.candidate_root()?,
        record.conditional_approval.as_ref(),
    ) {
        Ok(recovery) => {
            record.status = if record.conditional_approval.is_some() {
                RunStatus::PublishedConditional
            } else {
                RunStatus::Published
            };
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

fn parse_thread_members(args: ThreadOpenArgs) -> Result<Vec<ThreadMemberRequest>, ClewError> {
    let mut services = std::collections::BTreeMap::new();
    for value in args.service_aliases {
        let (alias, service) = parse_binding(&value, "service alias")?;
        if services
            .insert(alias.to_owned(), service.to_owned())
            .is_some()
        {
            return Err(invalid("duplicate thread service-alias binding"));
        }
    }
    let mut requests = Vec::with_capacity(args.members.len());
    let mut aliases = std::collections::BTreeSet::new();
    for value in args.members {
        let (alias, session_id) = parse_binding(&value, "member")?;
        if !aliases.insert(alias.to_owned()) {
            return Err(invalid("duplicate thread member alias"));
        }
        let service_alias = services.remove(alias).unwrap_or_else(|| alias.to_owned());
        requests.push(ThreadMemberRequest {
            member_alias: alias.to_owned(),
            service_alias,
            session_id: session_id.to_owned(),
        });
    }
    if !services.is_empty() {
        return Err(invalid(
            "thread service-alias override has no matching member",
        ));
    }
    Ok(requests)
}

fn parse_binding<'a>(value: &'a str, label: &str) -> Result<(&'a str, &'a str), ClewError> {
    let (left, right) = value
        .split_once('=')
        .ok_or_else(|| invalid(&format!("thread {label} must use ALIAS=VALUE")))?;
    if left.is_empty() || right.is_empty() {
        return Err(invalid(&format!(
            "thread {label} must have non-empty ALIAS and VALUE"
        )));
    }
    Ok((left, right))
}

fn reject_thread_context_id(context_id: &str) -> Result<(), ClewError> {
    reject_thread_coverage_id(context_id)?;
    if context_id.starts_with("thread-context:") {
        return Err(ClewError::new(
            ErrorCode::PreconditionFailed,
            "thread contexts are read-only analysis evidence and cannot be used for plan validation or task execution",
        ));
    }
    Ok(())
}

fn reject_thread_session_id(session_id: &str) -> Result<(), ClewError> {
    reject_thread_coverage_id(session_id)?;
    if session_id.starts_with("thread:") {
        return Err(ClewError::new(
            ErrorCode::PreconditionFailed,
            "threads are read-only analysis authorities and cannot be used by mutation or publication commands",
        ));
    }
    Ok(())
}

fn reject_thread_coverage_id(value: &str) -> Result<(), ClewError> {
    if value.starts_with("thread-coverage:") {
        return Err(ClewError::new(
            ErrorCode::PreconditionFailed,
            "thread coverage is read-only analysis evidence and cannot authorize mutation or publication",
        ));
    }
    Ok(())
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

fn require_record_prepared_authority(
    record: &RunRecord,
    prepared: &clew::task_run_v2::PreparedCandidateV2,
) -> Result<(), ClewError> {
    if record.context_id != prepared.context_id
        || record.plan_id != prepared.plan_id
        || record.candidate_commit.as_deref() != Some(prepared.candidate_commit.as_str())
        || record.candidate_snapshot.as_ref() != Some(&prepared.candidate_snapshot)
        || record.prepared_authority_digest.as_deref()
            != Some(prepared.prepared_authority_digest.as_str())
        || record
            .conditional_approval
            .as_ref()
            .is_some_and(|approval| {
                approval.run_id != record.run_id
                    || approval.request_digest != record.request_digest
                    || approval.context_id != record.context_id
                    || approval.plan_id != record.plan_id
                    || approval.candidate_commit != prepared.candidate_commit
                    || approval.candidate_snapshot != prepared.candidate_snapshot
                    || approval.prepared_authority_digest != prepared.prepared_authority_digest
            })
    {
        return Err(ClewError::new(
            ErrorCode::PreconditionFailed,
            "prepared or conditional approval authority differs from immutable run authority",
        ));
    }
    Ok(())
}

fn process_start_token(pid: u32) -> Result<Option<String>, ClewError> {
    #[cfg(target_os = "macos")]
    {
        let mut info: libc::proc_bsdinfo = unsafe { std::mem::zeroed() };
        let size = std::mem::size_of::<libc::proc_bsdinfo>();
        let result = unsafe {
            libc::proc_pidinfo(
                pid as i32,
                libc::PROC_PIDTBSDINFO,
                0,
                (&mut info as *mut libc::proc_bsdinfo).cast(),
                size as i32,
            )
        };
        if result != size as i32 || info.pbi_pid != pid {
            return Ok(None);
        }
        Ok(Some(format!(
            "{}:{}",
            info.pbi_start_tvsec, info.pbi_start_tvusec
        )))
    }
    #[cfg(target_os = "linux")]
    {
        let stat = match std::fs::read_to_string(format!("/proc/{pid}/stat")) {
            Ok(stat) => stat,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(io_error(error)),
        };
        let fields = stat
            .get(
                stat.rfind(')')
                    .ok_or_else(|| internal("invalid process stat"))?
                    + 1..,
            )
            .ok_or_else(|| internal("invalid process stat"))?
            .split_whitespace()
            .collect::<Vec<_>>();
        Ok(fields.get(19).map(|start| (*start).to_owned()))
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
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
    // The supervisor handles TERM cooperatively: it closes worker-spawn
    // admission, terminates every registered worker group in parallel, and
    // exits. Its worker grace period is two seconds, so keep a bounded margin
    // before the supervisor's own group is escalated to KILL.
    for _ in 0..100 {
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

fn read_json<T: DeserializeOwned>(
    state: &clew::state::StateAuthority,
    path: &Path,
    limit: usize,
) -> Result<T, ClewError> {
    serde_json::from_slice(
        &state
            .read_private_file(path, limit)
            .map_err(|_| invalid("managed JSON is missing, unsafe, or oversized"))?,
    )
    .map_err(parse_error)
}

fn read_bounded_regular_file(
    path: &Path,
    limit: usize,
    failure_message: &str,
) -> Result<Vec<u8>, ClewError> {
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    options.custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK);
    let mut file = options.open(path).map_err(|_| invalid(failure_message))?;
    let metadata = file.metadata().map_err(|_| invalid(failure_message))?;
    if !metadata.is_file() || metadata.len() > limit as u64 {
        return Err(invalid(failure_message));
    }
    let capacity = usize::try_from(metadata.len()).map_err(|_| invalid(failure_message))?;
    let mut bytes = Vec::with_capacity(capacity);
    file.by_ref()
        .take(limit.saturating_add(1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|_| invalid(failure_message))?;
    if bytes.len() > limit {
        return Err(invalid(failure_message));
    }
    Ok(bytes)
}

fn read_private_diagnostic_input(path: &Path, limit: usize) -> Result<Vec<u8>, ClewError> {
    if !path.is_absolute()
        || path.components().any(|component| {
            matches!(
                component,
                std::path::Component::CurDir | std::path::Component::ParentDir
            )
        })
    {
        return Err(invalid(
            "diagnostic input must use a normalized absolute path",
        ));
    }
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    options.custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK);
    let mut file = options
        .open(path)
        .map_err(|_| invalid("diagnostic input is unavailable or unsafe"))?;
    let metadata = file
        .metadata()
        .map_err(|_| invalid("diagnostic input is unavailable or unsafe"))?;
    #[cfg(unix)]
    let private = metadata.uid() == unsafe { libc::geteuid() }
        && metadata.permissions().mode() & 0o777 == 0o600;
    #[cfg(not(unix))]
    let private = false;
    if !metadata.is_file() || !private || metadata.len() > limit as u64 {
        return Err(invalid(
            "diagnostic input must be a caller-owned mode-0600 regular file within the size limit",
        ));
    }
    let capacity = usize::try_from(metadata.len())
        .map_err(|_| invalid("diagnostic input exceeds the host size"))?;
    let mut bytes = Vec::with_capacity(capacity);
    file.by_ref()
        .take(limit.saturating_add(1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|_| invalid("diagnostic input changed or became unreadable"))?;
    let after = file
        .metadata()
        .map_err(|_| invalid("diagnostic input changed or became unreadable"))?;
    #[cfg(unix)]
    let unchanged = metadata.dev() == after.dev()
        && metadata.ino() == after.ino()
        && metadata.mode() == after.mode()
        && metadata.uid() == after.uid()
        && metadata.len() == after.len()
        && metadata.mtime() == after.mtime()
        && metadata.mtime_nsec() == after.mtime_nsec()
        && metadata.ctime() == after.ctime()
        && metadata.ctime_nsec() == after.ctime_nsec();
    #[cfg(not(unix))]
    let unchanged = false;
    if bytes.len() != capacity || !unchanged {
        return Err(invalid("diagnostic input changed while it was read"));
    }
    Ok(bytes)
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
        for removed in ["project", "index", "resolve", "thread", "task-apply"] {
            assert!(Cli::try_parse_from(["clew", removed]).is_err());
        }
    }

    #[test]
    fn operational_entrypoints_require_explicit_closed_arguments() {
        assert!(Cli::try_parse_from(["clew", "capabilities"]).is_ok());
        assert!(Cli::try_parse_from(["clew", "capabilities", "--human"]).is_ok());
        assert!(Cli::try_parse_from(["clew", "doctor"]).is_ok());
        assert!(Cli::try_parse_from(["clew", "doctor", "--human"]).is_ok());
        assert!(Cli::try_parse_from(["clew", "upgrade"]).is_ok());
        assert!(Cli::try_parse_from(["clew", "upgrade", "--human"]).is_err());
        assert!(Cli::try_parse_from(["clew", "--json", "capabilities"]).is_err());
        assert!(Cli::try_parse_from(["clew", "doctor", "--json"]).is_err());
        assert!(
            Cli::try_parse_from([
                "clew",
                "change",
                "check-freshness",
                "--session",
                "session:fixture",
            ])
            .is_ok()
        );
        assert!(
            Cli::try_parse_from([
                "clew",
                "support",
                "summarize",
                "--input",
                "/private/fixture.json",
            ])
            .is_ok()
        );
        assert!(Cli::try_parse_from(["clew", "support", "summarize"]).is_err());
        let java = Cli::try_parse_from([
            "clew",
            "session",
            "open",
            "--repo",
            ".",
            "--target-ref",
            "refs/heads/main",
            "--language",
            "java",
            "--compilation",
            ":/main",
        ]);
        assert!(java.is_ok());
    }

    #[test]
    fn source_checkout_upgrade_explains_the_supported_update_path() {
        let cli = Cli::try_parse_from(["clew", "upgrade"]).unwrap();
        assert_eq!(OutputMode::from_cli(&cli), OutputMode::HumanUpgrade);
        let error = run(cli).unwrap_err();
        assert_eq!(error.code, ErrorCode::InvalidInput);
        assert!(error.message.contains("source checkout"));
        assert!(error.message.contains("Git"));
    }

    #[test]
    fn human_capabilities_is_readable_and_preserves_the_support_boundary() {
        let value = json!({
            "status":"PILOT_READY",
            "runtimeMode":"RELEASE",
            "supportMatrix":{
                "operatingSystems":["linux","macos"],
                "profiles":[
                    {
                        "buildSystem":"GRADLE_WRAPPER",
                        "compilerVersion":"2.4.10",
                        "language":"kotlin",
                        "mutation":true,
                        "status":"PILOT_READY"
                    },
                    {
                        "engineVersion":"tree-sitter-python-0.25.0",
                        "language":"python",
                        "mutation":true,
                        "status":"PILOT_READY"
                    },
                    {
                        "buildSystem":"MAVEN",
                        "compilerVersion":"21",
                        "language":"java",
                        "mutation":false,
                        "status":"READ_ONLY_PREVIEW"
                    }
                ],
                "threads":{
                    "minimumMembers":2,
                    "maximumMembers":8,
                    "status":"READ_ONLY_ANALYSIS"
                }
            },
            "packagedWorkers":[
                {"compilerVersion":"2.4.10","runtimeName":"kotlin24"}
            ]
        });
        let report = human_capabilities(&value);
        assert!(report.contains("Codeclew capabilities"));
        assert!(report.contains("Kotlin 2.4.10 / Gradle wrapper: read and change"));
        assert!(report.contains("Python tree-sitter-python-0.25.0: read and change"));
        assert!(report.contains("Java 21 / Maven: read only (read only preview)"));
        assert!(report.contains("2-8 members"));
        assert!(report.contains("Run without --human for canonical JSON"));
        assert!(!report.contains("codeclew-capabilities/1.0"));
    }

    #[test]
    fn human_doctor_explains_required_remediation_without_private_identity() {
        let value = json!({
            "status":"ACTION_REQUIRED",
            "runtimeMode":"RELEASE",
            "checks":[
                {
                    "checkId":"tool.git",
                    "required":true,
                    "status":"PASS",
                    "remediationId":null
                },
                {
                    "checkId":"repository.clean",
                    "required":true,
                    "status":"ACTION_REQUIRED",
                    "remediationId":"CLEAN_TARGET_WORKTREE"
                }
            ]
        });
        let report = human_doctor(&value);
        assert!(report.contains("Status: ACTION REQUIRED"));
        assert!(report.contains("[PASS] Git is available (required)"));
        assert!(report.contains("[ACTION_REQUIRED] Target worktree is clean (required)"));
        assert!(report.contains("commit, stash, or use a separate clean worktree"));
        assert!(!report.contains("/private/"));
    }

    #[test]
    fn change_facade_requires_explicit_authorities() {
        assert!(
            Cli::try_parse_from([
                "clew",
                "change",
                "open",
                "--repo",
                ".",
                "--target-ref",
                "main",
                "--language",
                "kotlin",
                "--compilation",
                ":/main",
                "--intent",
                "change total",
                "--term",
                "total",
            ])
            .is_ok()
        );
        assert!(
            Cli::try_parse_from([
                "clew",
                "change",
                "open",
                "--repo",
                ".",
                "--target-ref",
                "main",
                "--compilation",
                ":/main",
                "--intent",
                "change total",
            ])
            .is_err()
        );
        assert!(
            Cli::try_parse_from([
                "clew",
                "change",
                "prepare",
                "--session",
                "session:authority",
                "--context",
                "context:authority",
            ])
            .is_err()
        );
    }

    #[test]
    fn change_publish_preserves_conditional_approval_binding() {
        assert!(
            Cli::try_parse_from([
                "clew",
                "change",
                "publish",
                "--session",
                "session:authority",
                "--run",
                "run:request",
                "--acknowledge-obligation",
                "context:sha256:authority",
            ])
            .is_err()
        );
        assert!(
            Cli::try_parse_from([
                "clew",
                "change",
                "publish",
                "--session",
                "session:authority",
                "--run",
                "run:request",
                "--allow-conditional",
                "--prepared-authority-digest",
                "sha256:authority",
                "--acknowledge-obligation",
                "context:sha256:authority",
            ])
            .is_ok()
        );
    }

    #[test]
    fn change_open_failure_is_compensated_or_session_bound() {
        let original = ClewError::new(ErrorCode::SymbolNotFound, "missing symbol");
        let compensated = change_open_failure(original.clone(), "session:authority", || Ok(()));
        assert_eq!(compensated.code, original.code);
        assert_eq!(compensated.message, original.message);
        assert_eq!(compensated.transaction_id, None);

        let recovery = change_open_failure(original, "session:authority", || {
            Err(ClewError::new(ErrorCode::StateCorrupt, "cleanup failed"))
        });
        assert_eq!(recovery.code, ErrorCode::TransactionRecoveryRequired);
        assert_eq!(
            recovery.transaction_id.as_deref(),
            Some("session:authority")
        );
        assert!(recovery.retryable);
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
        assert!(cancellation_allowed(RunStatus::ReadyToPublish));
        assert!(cancellation_allowed(RunStatus::ReadyToPublishConditional));
        assert!(!cancellation_allowed(RunStatus::Publishing));
        assert!(!cancellation_allowed(RunStatus::Published));
        assert!(!cancellation_allowed(RunStatus::WorktreeRecoveryRequired));
    }

    #[test]
    fn session_lifecycle_commands_are_explicit() {
        for command in ["close", "abort"] {
            assert!(
                Cli::try_parse_from(
                    ["clew", "session", command, "--session", "session:authority",]
                )
                .is_ok()
            );
        }
        assert!(
            Cli::try_parse_from([
                "clew",
                "session",
                "relocate",
                "--session",
                "session:authority",
                "--repo",
                ".",
            ])
            .is_ok()
        );
        assert!(
            Cli::try_parse_from([
                "clew",
                "session",
                "gc",
                "--session",
                "session:authority",
                "--force",
            ])
            .is_ok()
        );
    }

    #[test]
    fn conditional_publish_requires_explicit_flag_and_qualified_obligation() {
        assert!(
            Cli::try_parse_from([
                "clew",
                "session",
                "publish",
                "--session",
                "session:authority",
                "--run",
                "run:request",
                "--acknowledge-obligation",
                "context:sha256:authority",
            ])
            .is_err()
        );
        assert!(
            Cli::try_parse_from([
                "clew",
                "session",
                "publish",
                "--session",
                "session:authority",
                "--run",
                "run:request",
                "--allow-conditional",
                "--prepared-authority-digest",
                "sha256:authority",
                "--acknowledge-obligation",
                "context:sha256:authority",
            ])
            .is_ok()
        );
    }

    #[test]
    fn session_open_requires_explicit_language_and_compilation_authority() {
        assert!(
            Cli::try_parse_from([
                "clew",
                "session",
                "open",
                "--repo",
                ".",
                "--target-ref",
                "main",
            ])
            .is_err()
        );
        let parsed = Cli::try_parse_from([
            "clew",
            "session",
            "open",
            "--repo",
            ".",
            "--target-ref",
            "main",
            "--language",
            "kotlin",
            "--compilation",
            ":workers:kotlin/main",
            "--compilation",
            ":workers:kotlin23/main",
        ])
        .unwrap();
        let Command::Session {
            command: SessionCommand::Open(args),
        } = parsed.command
        else {
            panic!("expected session open command");
        };
        assert_eq!(
            args.compilation,
            [":workers:kotlin/main", ":workers:kotlin23/main"]
        );
        assert!(matches!(args.language, SessionLanguageArg::Kotlin));
        assert!(
            Cli::try_parse_from([
                "clew",
                "session",
                "open",
                "--repo",
                ".",
                "--target-ref",
                "main",
                "--language",
                "python",
                "--compilation",
                "python:.#backend",
            ])
            .is_ok()
        );
        assert!(
            Cli::try_parse_from([
                "clew",
                "session",
                "open",
                "--repo",
                ".",
                "--target-ref",
                "main",
                "--language",
                "rust",
                "--compilation",
                "cargo:crates/clew/Cargo.toml#clew#lib#clew",
            ])
            .is_ok()
        );
    }

    #[test]
    fn thread_surface_is_closed_and_bindings_are_explicit() {
        let parsed = Cli::try_parse_from([
            "clew",
            "thread",
            "open",
            "--member",
            "api=session:one",
            "--member",
            "client=session:two",
            "--service-alias",
            "api=orders",
        ])
        .unwrap();
        let Command::Thread {
            command: ThreadCommand::Open(args),
        } = parsed.command
        else {
            panic!("expected thread open command");
        };
        let requests = parse_thread_members(args).unwrap();
        assert_eq!(requests[0].member_alias, "api");
        assert_eq!(requests[0].service_alias, "orders");
        assert_eq!(requests[1].service_alias, "client");
        for command in ["inspect", "publish"] {
            assert!(Cli::try_parse_from(["clew", "thread", command]).is_err());
        }
        assert!(
            reject_thread_context_id(
                "thread-context:sha256:ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"
            )
            .is_err()
        );
        assert!(reject_thread_session_id("thread:authority").is_err());

        let parsed = Cli::try_parse_from([
            "clew",
            "thread",
            "callables",
            "--thread",
            "thread:one",
            "--context",
            "thread-context:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "--task-id",
            "task-01",
            "--pair-id",
            "pair-01",
            "--provider",
            "api",
            "--consumer",
            "client",
            "--term",
            "callable:sample/Api.read#jvm:()I",
        ])
        .unwrap();
        let Command::Thread {
            command: ThreadCommand::Callables(args),
        } = parsed.command
        else {
            panic!("expected thread callables command");
        };
        assert_eq!(args.provider, "api");
        assert_eq!(args.consumer, "client");
        assert_eq!(args.terms.len(), 1);

        let parsed = Cli::try_parse_from([
            "clew",
            "thread",
            "impact",
            "--thread",
            "thread:one",
            "--fact-set",
            "thread-callables:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "--pair-id",
            "pair-01",
            "--subject-kind",
            "full-symbol",
            "--subject",
            "callable:sample/Api.read#jvm:()I",
            "--member",
            "api",
        ])
        .unwrap();
        let Command::Thread {
            command: ThreadCommand::Impact(args),
        } = parsed.command
        else {
            panic!("expected thread impact command");
        };
        assert!(matches!(
            args.subject_kind,
            ThreadImpactSubjectKindArg::FullSymbol
        ));
        assert_eq!(args.member.as_deref(), Some("api"));
        assert!(
            Cli::try_parse_from([
                "clew",
                "thread",
                "impact",
                "--thread",
                "thread:one",
                "--fact-set",
                "thread-callables:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "--pair-id",
                "pair-01",
                "--subject-kind",
                "full-symbol",
                "--subject",
                "callable:sample/Api.read#jvm:()I",
            ])
            .is_err()
        );
        for kind in ["callable-family", "token"] {
            assert!(
                Cli::try_parse_from([
                    "clew",
                    "thread",
                    "impact",
                    "--thread",
                    "thread:one",
                    "--fact-set",
                    "thread-callables:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                    "--pair-id",
                    "pair-01",
                    "--subject-kind",
                    kind,
                    "--subject",
                    "Api",
                ])
                .is_ok()
            );
        }
        let parsed = Cli::try_parse_from([
            "clew",
            "thread",
            "impact",
            "--thread",
            "thread:one",
            "--fact-set",
            "thread-callables:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "--pair-id",
            "pair-01",
            "--subject-kind",
            "token",
            "--subject",
            "Api",
            "--member",
            "api",
            "--declarations-only",
            "--declaration-name-only",
        ])
        .unwrap();
        let Command::Thread {
            command: ThreadCommand::Impact(args),
        } = parsed.command
        else {
            panic!("expected thread impact command");
        };
        assert!(matches!(
            args.subject_kind,
            ThreadImpactSubjectKindArg::Token
        ));
        assert_eq!(args.member.as_deref(), Some("api"));
        assert!(args.declarations_only);
        assert!(args.declaration_name_only);

        let parsed = Cli::try_parse_from([
            "clew",
            "thread",
            "validate",
            "--before-thread",
            "thread:before",
            "--before-impact",
            "thread-impact:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "--after-thread",
            "thread:after",
            "--after-impact",
            "thread-impact:sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            "--member-correspondence",
            "provider-old=provider-new",
            "--member-correspondence",
            "consumer-old=consumer-new",
            "--coverage",
            "coverage.json",
        ])
        .unwrap();
        let Command::Thread {
            command: ThreadCommand::Validate(args),
        } = parsed.command
        else {
            panic!("expected thread validate command");
        };
        assert_eq!(args.member_correspondence.len(), 2);
    }

    #[test]
    fn workspace_surface_is_thin_and_manifest_driven() {
        let parsed = Cli::try_parse_from([
            "clew",
            "workspace",
            "open",
            "--catalog",
            "/private/catalog.json",
        ])
        .unwrap();
        let Command::Workspace {
            command: WorkspaceCommand::Open(args),
        } = parsed.command
        else {
            panic!("expected workspace open command");
        };
        assert_eq!(args.catalog, PathBuf::from("/private/catalog.json"));
        assert!(
            Cli::try_parse_from([
                "clew",
                "workspace",
                "context",
                "--workspace",
                "workspace:authority",
                "--intent",
                "inspect both repositories",
                "--term",
                "Service",
            ])
            .is_ok()
        );
        assert!(
            Cli::try_parse_from([
                "clew",
                "workspace",
                "prepare",
                "--workspace",
                "workspace:authority",
                "--request",
                "/private/prepare.json",
            ])
            .is_ok()
        );
        assert!(
            Cli::try_parse_from([
                "clew",
                "workspace",
                "observe",
                "--workspace",
                "workspace:authority",
                "--request",
                "/private/scenario.json",
                "--evidence",
                "/private/raw.json",
            ])
            .is_ok()
        );
        assert!(
            Cli::try_parse_from([
                "clew",
                "workspace",
                "publish",
                "--workspace",
                "workspace:authority",
                "--request",
                "/private/publication.json",
            ])
            .is_ok()
        );
        assert!(
            Cli::try_parse_from([
                "clew",
                "workspace",
                "recover",
                "--workspace",
                "workspace:authority",
                "--publication",
                "workspace-publication:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            ])
            .is_ok()
        );
        assert!(Cli::try_parse_from(["clew", "workspace", "gc"]).is_err());
    }

    #[test]
    fn thread_coverage_ids_are_rejected_before_mutation_lookup() {
        let coverage = "thread-coverage:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        assert_eq!(
            reject_thread_coverage_id(coverage).unwrap_err().code,
            ErrorCode::PreconditionFailed
        );
        assert!(reject_thread_context_id(coverage).is_err());
        assert!(reject_thread_session_id(coverage).is_err());
        assert!(task_run_status(coverage).is_err());
    }

    #[test]
    fn bounded_regular_file_reader_is_exact_and_path_safe() {
        let temporary = tempfile::tempdir().unwrap();
        let exact = temporary.path().join("exact.json");
        std::fs::write(&exact, b"1234").unwrap();
        assert_eq!(
            read_bounded_regular_file(&exact, 4, "unsafe input").unwrap(),
            b"1234"
        );
        assert!(read_bounded_regular_file(&exact, 3, "unsafe input").is_err());
        assert!(
            read_bounded_regular_file(&temporary.path().join("missing"), 4, "unsafe input")
                .is_err()
        );
        #[cfg(unix)]
        {
            use std::ffi::CString;
            use std::os::unix::ffi::OsStrExt;

            std::os::unix::fs::symlink(&exact, temporary.path().join("link.json")).unwrap();
            assert!(
                read_bounded_regular_file(&temporary.path().join("link.json"), 4, "unsafe input")
                    .is_err()
            );
            let fifo = temporary.path().join("coverage.fifo");
            let fifo_name = CString::new(fifo.as_os_str().as_bytes()).unwrap();
            assert_eq!(unsafe { libc::mkfifo(fifo_name.as_ptr(), 0o600) }, 0);
            let started = std::time::Instant::now();
            assert!(read_bounded_regular_file(&fifo, 4, "unsafe input").is_err());
            assert!(started.elapsed() < std::time::Duration::from_secs(1));
        }
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
