#![allow(dead_code, unused_imports)]

use clap::{ArgGroup, Args, Parser, Subcommand, ValueEnum};
use clew::canonical;
use clew::error::{ClewError, ErrorCode};
use clew::evidence_authority::{
    EvidenceAuthority, MAP_EDGE_WITH_CONTEXT_DECISION_SCHEMA, MapEdgeWithContextDecision,
    TYPED_GOAL_BINDING_DECISION_SCHEMA, TYPED_GOAL_BINDING_REQUEST_SCHEMA,
    TypedGoalBindingDecision, TypedGoalBindingRequest, TypedGoalRefusal, TypedGoalRefusalReason,
};
use clew::graph;
use clew::index::{REPOSITORY_INDEX_FACT, RepositoryIndex};
use clew::model::*;
use clew::projection::{
    self, BoundaryPolicy, ProjectionBudget, ProjectionLevel, ProjectionQuery, ThreadKind, Traversal,
};
use clew::proto::RequestKind;
use clew::semantic_goal::{
    SemanticGoal, TYPED_GOAL_MAX_REQUEST_BYTES, TypedGoalLanguageError, typed_goal_language_schema,
};
use clew::session::{
    ModelCachePolicy, RunRecord, RunStatus, SessionAuthority, bounded_context_stdout,
};
use clew::task_context;
use clew::thread_projection;
use clew::transaction;
use clew::worker::{WorkerClient, inherited_build_state_root, workspace_root};
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

#[derive(Parser)]
#[command(
    name = "clew",
    version,
    about = "Codeclew managed semantic change runtime"
)]
struct Cli {
    #[arg(
        long,
        global = true,
        help = "Emit stable machine-readable JSON (JSON is also the default)"
    )]
    json: bool,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    #[cfg(any())]
    Doctor,
    #[cfg(any())]
    #[command(about = "Emit a deterministic machine-readable product schema")]
    Schema {
        #[command(subcommand)]
        command: SchemaCommand,
    },
    #[cfg(any())]
    Project {
        #[command(subcommand)]
        command: ProjectCommand,
    },
    #[cfg(any())]
    Index(IndexArgs),
    #[cfg(any())]
    Resolve {
        #[command(subcommand)]
        command: ResolveCommand,
    },
    #[cfg(any())]
    Cfg(SymbolArgs),
    #[cfg(any())]
    Slice(SliceArgs),
    #[cfg(any())]
    #[command(about = "Query a bounded, evidence-backed semantic view without filesystem search")]
    Projection(ProjectionArgs),
    #[cfg(any())]
    #[command(about = "Prove a source-free semantic change plan from live compiler evidence")]
    Prove {
        #[command(subcommand)]
        command: ProveCommand,
    },
    #[cfg(any())]
    #[command(about = "Apply an authority-proved semantic change as an atomic commit")]
    Apply {
        #[command(subcommand)]
        command: ApplyCommand,
    },
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
    #[cfg(any())]
    Edit {
        #[command(subcommand)]
        command: EditCommand,
    },
    #[cfg(any())]
    Tx {
        #[command(subcommand)]
        command: TxCommand,
    },
    #[command(name = "__task-run-execute", hide = true)]
    InternalTaskRunExecute(InternalTaskRunArgs),
}

#[derive(Subcommand)]
enum SessionCommand {
    Open(SessionOpenArgs),
    Inspect(SessionIdArgs),
    Publish(SessionPublishArgs),
    Recover(SessionPublishArgs),
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

#[derive(Subcommand)]
enum SchemaCommand {
    #[command(
        name = "typed-goal",
        about = "Emit the typed-goal constraint language registry"
    )]
    TypedGoal,
}

#[derive(Subcommand)]
enum ProjectCommand {
    Inspect(RepoArgs),
}
#[derive(Subcommand)]
enum ResolveCommand {
    Symbol(SymbolArgs),
    Expression(ExpressionArgs),
}
#[derive(Subcommand)]
enum EditCommand {
    Preview(PreviewArgs),
}
#[derive(Subcommand)]
enum TxCommand {
    Validate(TxFileArgs),
    Commit(CommitArgs),
    Inspect(InspectTxArgs),
}
#[derive(Subcommand)]
enum ProveCommand {
    #[command(
        name = "typed-goal",
        about = "Bind a family-neutral typed constraint goal from compiler evidence"
    )]
    TypedGoal(TypedGoalArgs),
    #[command(
        name = "map-edge-with-context",
        about = "Bind a typed collection-edge change and prove its preservation invariants"
    )]
    MapEdgeWithContext(MapEdgeWithContextArgs),
}
#[derive(Subcommand)]
enum ApplyCommand {
    #[command(
        name = "map-edge-with-context",
        about = "Prove and atomically materialize a typed collection-edge change"
    )]
    MapEdgeWithContext(ApplyMapEdgeWithContextArgs),
}

#[derive(Args)]
struct RepoArgs {
    #[arg(long)]
    repo: PathBuf,
    #[arg(long)]
    compilation: Option<String>,
}
#[derive(Args)]
struct IndexArgs {
    #[arg(long)]
    repo: PathBuf,
    #[arg(long)]
    compilation: Option<String>,
    #[arg(
        long,
        help = "Build the cold syntax/declaration index without K2 enrichment"
    )]
    syntax_only: bool,
    #[arg(long = "file", help = "Incrementally update one relative Kotlin file")]
    files: Vec<String>,
}
#[derive(Args)]
struct SymbolArgs {
    #[arg(long)]
    repo: PathBuf,
    #[arg(long, default_value = ":/main")]
    compilation: String,
    #[arg(long)]
    symbol: String,
}
#[derive(Args)]
struct ExpressionArgs {
    #[arg(long)]
    repo: PathBuf,
    #[arg(long)]
    file: String,
    #[arg(long)]
    offset: usize,
}
#[derive(Args)]
struct SliceArgs {
    #[arg(long)]
    repo: PathBuf,
    #[arg(long, default_value = ":/main")]
    compilation: String,
    #[arg(long, required_unless_present = "file", conflicts_with = "file")]
    symbol: Option<String>,
    #[arg(long, requires = "offset", conflicts_with = "symbol")]
    file: Option<String>,
    #[arg(long, requires = "file")]
    offset: Option<usize>,
    #[arg(long, value_enum, default_value = "both")]
    direction: DirectionArg,
    #[arg(long, default_value_t = 200)]
    max_nodes: usize,
    #[arg(long)]
    output: Option<PathBuf>,
}
#[derive(Args)]
struct ProjectionArgs {
    #[arg(long)]
    repo: PathBuf,
    #[arg(long, default_value = ":/main")]
    compilation: String,
    #[arg(long, required_unless_present = "file", conflicts_with = "file")]
    symbol: Option<String>,
    #[arg(long, requires = "offset", conflicts_with = "symbol")]
    file: Option<String>,
    #[arg(long, requires = "file")]
    offset: Option<usize>,
    #[arg(long, value_enum, default_value = "both")]
    direction: DirectionArg,
    #[arg(long, value_enum, default_value = "l4")]
    level: ProjectionLevelArg,
    #[arg(long = "thread", value_enum, default_value = "data")]
    thread_kind: ProjectionThreadKindArg,
    #[arg(long, default_value_t = 200)]
    max_nodes: usize,
    #[arg(long, default_value_t = 32 * 1024)]
    max_bytes: usize,
    #[arg(long)]
    claim: Option<String>,
    #[arg(long)]
    refuse_on_boundary: bool,
    #[arg(long)]
    output: Option<PathBuf>,
}
#[derive(Args)]
struct MapEdgeWithContextArgs {
    #[arg(long)]
    repo: PathBuf,
    #[arg(long, default_value = ":/main")]
    compilation: String,
    #[arg(long = "workflow-symbol")]
    workflow_symbol: String,
    #[arg(long = "test-symbol")]
    test_symbol: String,
    #[arg(long = "test-compilation", default_value = ":/test")]
    test_compilation: String,
    #[arg(long, default_value_t = 200)]
    max_nodes: usize,
}
#[derive(Args)]
#[command(group(
    ArgGroup::new("request_input")
        .required(true)
        .multiple(false)
        .args(["request", "request_json"])
))]
struct TypedGoalArgs {
    #[arg(long)]
    repo: PathBuf,
    #[arg(long, conflicts_with = "request_json")]
    request: Option<PathBuf>,
    #[arg(long, conflicts_with = "request")]
    request_json: Option<String>,
    #[arg(long)]
    compilation: Option<String>,
    #[arg(
        long,
        help = "Bind REQUIRE_ORACLE to an immutable external-task-spec/0.1 document"
    )]
    external_spec: Option<PathBuf>,
}
#[derive(Args)]
struct ApplyMapEdgeWithContextArgs {
    #[command(flatten)]
    proof: MapEdgeWithContextArgs,
    #[arg(long)]
    target_ref: String,
    #[arg(long, default_value = "codeclew-semantic-agent")]
    actor: String,
    #[arg(long)]
    output: Option<PathBuf>,
}

#[derive(Args)]
struct SessionOpenArgs {
    #[arg(long)]
    repo: PathBuf,
    #[arg(long)]
    target_ref: String,
    #[arg(long, default_value = ":/main")]
    compilation: String,
    #[arg(long, value_enum, default_value = "non-cacheable")]
    model_cache: ModelCachePolicyArg,
}

#[derive(Clone, Copy, ValueEnum)]
enum ModelCachePolicyArg {
    NonCacheable,
    TrackedManifest,
    SealedExternal,
}

#[derive(Args)]
struct SessionIdArgs {
    #[arg(long)]
    session: String,
}

#[derive(Args)]
struct SessionPublishArgs {
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
#[derive(Clone, Copy, ValueEnum)]
enum DirectionArg {
    Forward,
    Backward,
    Both,
}
#[derive(Clone, Copy, ValueEnum)]
enum ProjectionLevelArg {
    L0,
    L1,
    L2,
    L3,
    L4,
    L5,
}
#[derive(Clone, Copy, ValueEnum)]
enum ProjectionThreadKindArg {
    Control,
    Data,
    Journey,
    State,
    Effect,
    Failure,
    Config,
    TestEvidence,
    Change,
}
#[derive(Args)]
struct PreviewArgs {
    #[arg(long)]
    repo: PathBuf,
    #[arg(long)]
    thread: PathBuf,
    #[arg(long = "operations")]
    edit: PathBuf,
    #[arg(long)]
    output: Option<PathBuf>,
}
#[derive(Args)]
struct TxFileArgs {
    #[arg(long)]
    repo: PathBuf,
    #[arg(long = "transaction")]
    file: PathBuf,
}
#[derive(Args)]
struct CommitArgs {
    #[arg(long)]
    repo: PathBuf,
    #[arg(long = "transaction")]
    file: PathBuf,
    #[arg(long)]
    target_ref: String,
}
#[derive(Args)]
struct InspectTxArgs {
    #[arg(long)]
    repo: PathBuf,
    #[arg(long)]
    transaction_id: String,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let stdout_budget = None;
    let started = std::time::Instant::now();
    let result = run(cli);
    let rendered_fits = |rendered: &str| {
        stdout_budget.is_none_or(|budget| rendered.len().saturating_add(1) <= budget)
    };
    let success = result
        .as_ref()
        .is_ok_and(|value| canonical::pretty(value).is_ok_and(|rendered| rendered_fits(&rendered)));
    eprintln!(
        "{}",
        serde_json::to_string(&json!({
            "event":"request_completed",
            "durationMs":started.elapsed().as_millis(),
            "success":success
        }))
        .unwrap_or_default()
    );
    match result {
        Ok(value) => {
            let rendered = canonical::pretty(&value).unwrap();
            if rendered_fits(&rendered) {
                println!("{rendered}");
                ExitCode::SUCCESS
            } else {
                ExitCode::from(exit_code(&ErrorCode::SliceBudgetExceeded))
            }
        }
        Err(error) => {
            let rendered =
                canonical::pretty(&json!({"schema":"semantic-error/0.1","error":error})).unwrap();
            if rendered_fits(&rendered) {
                println!("{rendered}");
            }
            ExitCode::from(exit_code(&error.code))
        }
    }
}

fn run(cli: Cli) -> Result<Value, ClewError> {
    let workspace = workspace_root();
    match cli.command {
        #[cfg(any())]
        Command::Doctor => {
            let compiler_index_root = None::<PathBuf>;
            let worker = start_cli_worker(&workspace, compiler_index_root.as_deref())?;
            let result = json!({"schema":"semantic-doctor/0.1","status":"OK","rustCore":env!("CARGO_PKG_VERSION"),"worker":{"language":worker.capabilities.language,"version":worker.capabilities.worker_version,"compilerVersion":worker.capabilities.compiler_version,"operations":worker.capabilities.supported_operations}});
            worker.shutdown()?;
            Ok(result)
        }
        #[cfg(any())]
        Command::Schema {
            command: SchemaCommand::TypedGoal,
        } => serde_json::to_value(typed_goal_language_schema()).map_err(parse_error),
        #[cfg(any())]
        Command::Project {
            command: ProjectCommand::Inspect(args),
        } => with_worker(&workspace, None, |w| {
            w.request(
                RequestKind::OpenProject,
                &json!({"repo":absolute(&args.repo)?,"compilation":args.compilation}),
            )
        }),
        #[cfg(any())]
        Command::Index(args) => with_worker(&workspace, None, |w| {
            let total_started = std::time::Instant::now();
            let repo = absolute(&args.repo)?;
            let index_started = std::time::Instant::now();
            let verified_facts = w.index_files_verified(
                &json!({"repo":repo,"compilation":args.compilation,"syntaxOnly":args.syntax_only,"files":args.files}),
            )?;
            let index_files_micros = index_started.elapsed().as_micros() as u64;
            let inspect_started = std::time::Instant::now();
            let facts = w.inspect_verified_index(&verified_facts)?;
            let inspect_receipt_micros = inspect_started.elapsed().as_micros() as u64;
            let compiler_index = w.last_profile.compiler_index.clone();
            let project_model_cache = w.last_profile.project_model_cache.clone();
            let worker_profile = w.last_profile.clone();
            let publication_started = std::time::Instant::now();
            let syntax_storage = args
                .syntax_only
                .then(|| format!("{}#syntax", args.compilation.as_deref().unwrap_or(":/main")));
            let mut index = RepositoryIndex::open_compilation(
                &repo,
                syntax_storage.as_deref().or(args.compilation.as_deref()),
            )?;
            let persistent_hash = index.update_verified(&verified_facts, w)?;
            let relation_snapshot = index.declaration_relations()?.ok_or_else(|| {
                ClewError::new(
                    ErrorCode::ProjectModelChanged,
                    "persistent index has no validated declaration relation snapshot",
                )
            })?;
            let descriptor_snapshot = index.declaration_descriptors()?.ok_or_else(|| {
                ClewError::new(
                    ErrorCode::ProjectModelChanged,
                    "persistent index has no validated declaration descriptor snapshot",
                )
            })?;
            let invalidations = index.invalidations()?;
            let freshness = index.freshness_status(REPOSITORY_INDEX_FACT)?;
            let repository_publication_micros = publication_started.elapsed().as_micros() as u64;
            Ok(json!({
                "schema":"semantic-index-result/0.1",
                "projectModelHash":facts["projectModelHash"],
                "workerIndexHash":facts["indexHash"],
                "persistentIndexHash":persistent_hash,
                "declarationRelations":relation_snapshot.graph,
                "declarationRelationHash":relation_snapshot.hash,
                "relationProvenance":relation_snapshot.provenance,
                "declarationDescriptors":descriptor_snapshot.graph,
                "declarationDescriptorHash":descriptor_snapshot.hash,
                "descriptorProvenance":descriptor_snapshot.provenance,
                "snapshotProvenance":{
                    "projectModelHash":facts["projectModelHash"],
                    "persistentIndexHash":persistent_hash,
                    "declarationRelationHash":relation_snapshot.hash,
                    "relationProvenance":relation_snapshot.provenance,
                    "declarationDescriptorHash":descriptor_snapshot.hash,
                    "descriptorProvenance":descriptor_snapshot.provenance,
                },
                "files":facts["files"].as_array().map_or(0,Vec::len),
                "invalidations":invalidations,
                "freshness":freshness,
                "compilerIndex":compiler_index,
                "projectModelCache":project_model_cache,
                "workerProfile":{
                    "serializationMicros":worker_profile.serialization_micros,
                    "ipcMicros":worker_profile.ipc_micros,
                    "workerProcessingMicros":worker_profile.worker_processing_micros,
                    "cacheRequests":worker_profile.cache_requests,
                    "cacheHits":worker_profile.cache_hits,
                    "psiParseMicros":worker_profile.psi_parse_micros,
                    "k2AnalysisMicros":worker_profile.k2_analysis_micros,
                    "firExtractionMicros":worker_profile.fir_extraction_micros,
                },
                "timing":{
                    "openProjectMicros":0,
                    "openProjectIncludedInIndexFiles":true,
                    "indexFilesMicros":index_files_micros,
                    "inspectReceiptMicros":inspect_receipt_micros,
                    "repositoryPublicationMicros":repository_publication_micros,
                    "totalMicros":total_started.elapsed().as_micros() as u64,
                },
            }))
        }),
        #[cfg(any())]
        Command::Resolve {
            command: ResolveCommand::Symbol(args),
        } => with_worker(&workspace, None, |w| {
            w.request(
                RequestKind::ResolveSymbol,
                &json!({"repo":absolute(&args.repo)?,"symbol":args.symbol,"compilation":args.compilation}),
            )
        }),
        #[cfg(any())]
        Command::Resolve {
            command: ResolveCommand::Expression(args),
        } => with_worker(&workspace, None, |w| {
            w.request(
                RequestKind::ResolveExpression,
                &json!({"repo":absolute(&args.repo)?,"file":args.file,"offset":args.offset}),
            )
        }),
        #[cfg(any())]
        Command::Cfg(args) => with_worker(&workspace, None, |w| {
            let raw = w.request(
                RequestKind::BuildLocalGraph,
                &json!({"repo":absolute(&args.repo)?,"symbol":args.symbol,"compilation":args.compilation}),
            )?;
            let graph: LocalGraph = serde_json::from_value(raw).map_err(parse_error)?;
            serde_json::to_value(graph::enrich(graph)).map_err(parse_error)
        }),
        #[cfg(any())]
        Command::Slice(args) => with_worker(&workspace, None, |w| slice_command(w, args)),
        #[cfg(any())]
        Command::Projection(args) => {
            with_worker(&workspace, None, |worker| projection_command(worker, args))
        }
        #[cfg(any())]
        Command::Prove {
            command: ProveCommand::TypedGoal(args),
        } => with_worker(&workspace, None, |worker| {
            let request = read_typed_goal_request(&args)?;
            if request.schema != TYPED_GOAL_BINDING_REQUEST_SCHEMA {
                return typed_goal_refusal_json(TypedGoalRefusalReason::InvalidGoal);
            }
            match request.goal.validate_executable() {
                Ok(()) => {}
                Err(TypedGoalLanguageError::UnsupportedConstraintDomain) => {
                    return typed_goal_refusal_json(
                        TypedGoalRefusalReason::UnsupportedConstraintDomain,
                    );
                }
                Err(_) => {
                    return typed_goal_refusal_json(TypedGoalRefusalReason::InvalidGoal);
                }
            }
            let repo = absolute(&args.repo)?;
            let revision = git_head(&repo)?;
            if request.goal.base_revision != revision {
                return typed_goal_refusal_json(TypedGoalRefusalReason::SnapshotMismatch);
            }
            let mut authority = EvidenceAuthority::open(&repo, &revision)?;
            let compilation = match (args.compilation.as_deref(), request.compilation.as_deref()) {
                (Some(cli), Some(requested)) if cli != requested => {
                    return typed_goal_refusal_json(TypedGoalRefusalReason::InvalidGoal);
                }
                (Some(cli), _) => Some(cli),
                (_, requested) => requested,
            };
            let decision = if let Some(specification) = args.external_spec.as_deref() {
                let receipt = match authority.issue_external_spec(
                    specification,
                    &request,
                    compilation,
                    worker,
                ) {
                    Ok(receipt) => receipt,
                    Err(_) => {
                        return typed_goal_refusal_json(
                            TypedGoalRefusalReason::ExternalSpecificationMismatch,
                        );
                    }
                };
                authority.bind_typed_goal_with_external_spec(
                    &request,
                    compilation,
                    &receipt,
                    worker,
                )?
            } else {
                authority.bind_typed_goal(&request.goal, &request.hints, compilation, worker)?
            };
            match decision {
                TypedGoalBindingDecision::Bound(receipt) => {
                    if !authority.recognizes_typed_goal(&receipt)?
                        || !authority.recognizes_typed_goal_summary(receipt.summary())?
                    {
                        return Err(ClewError::new(
                            ErrorCode::Internal,
                            "authority did not recognize the typed-goal proof it just issued",
                        ));
                    }
                    Ok(json!({
                        "schema": TYPED_GOAL_BINDING_DECISION_SCHEMA,
                        "status": "BOUND",
                        "proof": receipt.summary(),
                    }))
                }
                TypedGoalBindingDecision::Conditional(conditional) => {
                    serde_json::to_value(conditional).map_err(parse_error)
                }
                TypedGoalBindingDecision::Ambiguous(ambiguity) => {
                    serde_json::to_value(ambiguity).map_err(parse_error)
                }
                TypedGoalBindingDecision::Refused(refusal) => {
                    serde_json::to_value(refusal).map_err(parse_error)
                }
            }
        }),
        #[cfg(any())]
        Command::Prove {
            command: ProveCommand::MapEdgeWithContext(args),
        } => with_worker(&workspace, None, |worker| {
            let repo = absolute(&args.repo)?;
            let revision = git_head(&repo)?;
            let thread = build_thread(
                worker,
                SliceArgs {
                    repo: repo.clone(),
                    compilation: args.compilation,
                    symbol: Some(args.workflow_symbol),
                    file: None,
                    offset: None,
                    direction: DirectionArg::Both,
                    max_nodes: args.max_nodes,
                    output: None,
                },
            )?;
            let mut authority = EvidenceAuthority::open(&repo, &revision)?;
            let verified = authority.verify_thread(&thread, worker)?;
            let goal = SemanticGoal::map_edge_with_context(revision);
            match authority.bind_map_edge_with_context(
                &goal,
                &verified,
                &args.test_symbol,
                &args.test_compilation,
                worker,
            )? {
                MapEdgeWithContextDecision::Bound(receipt) => {
                    if !authority.recognizes_map_edge_with_context(&receipt)? {
                        return Err(ClewError::new(
                            ErrorCode::Internal,
                            "authority did not recognize the proof it just issued",
                        ));
                    }
                    Ok(json!({
                        "schema": MAP_EDGE_WITH_CONTEXT_DECISION_SCHEMA,
                        "status": "BOUND",
                        "proof": receipt.summary(),
                    }))
                }
                MapEdgeWithContextDecision::Conditional(conditional) => {
                    serde_json::to_value(conditional).map_err(parse_error)
                }
                MapEdgeWithContextDecision::Ambiguous(ambiguity) => {
                    serde_json::to_value(ambiguity).map_err(parse_error)
                }
                MapEdgeWithContextDecision::Refused(refusal) => {
                    serde_json::to_value(refusal).map_err(parse_error)
                }
            }
        }),
        #[cfg(any())]
        Command::Apply {
            command: ApplyCommand::MapEdgeWithContext(args),
        } => with_worker(&workspace, None, |worker| {
            let repo = absolute(&args.proof.repo)?;
            let revision = git_head(&repo)?;
            let thread = build_thread(
                worker,
                SliceArgs {
                    repo: repo.clone(),
                    compilation: args.proof.compilation,
                    symbol: Some(args.proof.workflow_symbol),
                    file: None,
                    offset: None,
                    direction: DirectionArg::Both,
                    max_nodes: args.proof.max_nodes,
                    output: None,
                },
            )?;
            let mut authority = EvidenceAuthority::open(&repo, &revision)?;
            let verified = authority.verify_thread(&thread, worker)?;
            let goal = SemanticGoal::map_edge_with_context(revision);
            match authority.bind_map_edge_with_context(
                &goal,
                &verified,
                &args.proof.test_symbol,
                &args.proof.test_compilation,
                worker,
            )? {
                MapEdgeWithContextDecision::Bound(receipt) => {
                    let proof = receipt.summary().clone();
                    let (result, transaction) = authority.commit_map_edge_with_context(
                        &receipt,
                        &args.actor,
                        &args.target_ref,
                        worker,
                    )?;
                    if let Some(output) = args.output.as_deref() {
                        write_artifact(output, &transaction)?;
                    }
                    Ok(json!({
                        "schema":"map-edge-with-context-apply/0.1",
                        "status":"COMMITTED",
                        "proof":proof,
                        "result":result,
                        "changedFiles":transaction.preview.as_ref().map(|preview| &preview.changed_files),
                        "finalCommit":transaction.final_commit,
                        "transactionArtifact":args.output,
                    }))
                }
                MapEdgeWithContextDecision::Conditional(conditional) => {
                    serde_json::to_value(conditional).map_err(parse_error)
                }
                MapEdgeWithContextDecision::Ambiguous(ambiguity) => {
                    serde_json::to_value(ambiguity).map_err(parse_error)
                }
                MapEdgeWithContextDecision::Refused(refusal) => {
                    serde_json::to_value(refusal).map_err(parse_error)
                }
            }
        }),
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
            Ok(json!({
                "schema":"codeclew-session-open/1.0",
                "status":"OPEN",
                "session":session,
            }))
        }
        Command::Session {
            command: SessionCommand::Inspect(args),
        } => {
            let (session, _) = SessionAuthority::load(&args.session)?;
            Ok(json!({"schema":"codeclew-session-inspect/1.0","session":session}))
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
            let context =
                session.store_context(None, args.intent, args.terms, projection, evidence)?;
            bounded_context_stdout(&context)
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
            let context =
                session.store_context(Some(args.context), intent, terms, projection, evidence)?;
            bounded_context_stdout(&context)
        }
        Command::Plan {
            command: PlanCommand::Validate(args),
        } => {
            let (session, _) = SessionAuthority::load(&args.session)?;
            let metadata = std::fs::symlink_metadata(&args.plan)
                .map_err(|error| ClewError::new(ErrorCode::InvalidInput, error.to_string()))?;
            if metadata.file_type().is_symlink()
                || !metadata.is_file()
                || metadata.len() as usize > clew::session::MAX_PLAN_BYTES
            {
                return Err(ClewError::new(
                    ErrorCode::InvalidInput,
                    "plan is missing, unsafe, or exceeds 1 MiB",
                ));
            }
            let source = std::fs::read(&args.plan)
                .map_err(|error| ClewError::new(ErrorCode::InvalidInput, error.to_string()))?;
            let plan = session.validate_plan(&args.context, &source)?;
            Ok(json!({
                "schema":"codeclew-plan-validation/1.0",
                "status":"VALID",
                "sessionId":session.session_id,
                "contextId":args.context,
                "planId":plan.plan_id,
                "sourceDigest":plan.source_digest,
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
        } => publish_task_run(&workspace, &args.session, &args.run),
        Command::Session {
            command: SessionCommand::Recover(args),
        } => recover_task_run(&workspace, &args.session, &args.run),
        Command::InternalTaskRunExecute(args) => execute_task_run(&workspace, &args.run),
        #[cfg(any())]
        Command::Edit {
            command: EditCommand::Preview(args),
        } => with_worker(&workspace, None, |w| {
            let repo = absolute(&args.repo)?;
            let thread: ThreadIr = read_json(&args.thread)?;
            let edit: EditIr = read_json(&args.edit)?;
            let report = transaction::preview(&repo, &thread, &edit, w)?;
            write_optional(args.output.as_deref(), &report)?;
            serde_json::to_value(report).map_err(parse_error)
        }),
        #[cfg(any())]
        Command::Tx {
            command: TxCommand::Validate(args),
        } => with_worker(&workspace, None, |w| {
            let repo = absolute(&args.repo)?;
            let mut tx: Transaction = read_json(&args.file)?;
            transaction::validate_required_threads(&tx)?;
            let transaction_id = tx.tx_id.clone();
            let snapshot_id = tx.thread.snapshot.index_snapshot.clone();
            let relevant = tx
                .edit
                .operations
                .first()
                .and_then(|operation| {
                    operation
                        .target
                        .get("anchorId")
                        .or_else(|| operation.target.get("ownerSymbolId"))
                })
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned();
            tx.status = "VALIDATING".into();
            transaction::ledger(&repo)?.append(&tx, "validation started")?;
            match transaction::preview(&repo, &tx.thread, &tx.edit, w) {
                Ok(report) => {
                    tx.preview = Some(report);
                    tx.status = "VALIDATED".into();
                    transaction::ledger(&repo)?.append(&tx, "validation passed")?;
                    serde_json::to_value(tx).map_err(parse_error)
                }
                Err(e) => {
                    tx.status = "VALIDATION_FAILED".into();
                    let _ = transaction::ledger(&repo)?.append(&tx, &e.message);
                    Err(e
                        .with_transaction(transaction_id)
                        .with_snapshot(snapshot_id)
                        .with_relevant(relevant))
                }
            }
        }),
        #[cfg(any())]
        Command::Tx {
            command: TxCommand::Commit(args),
        } => with_worker(&workspace, None, |w| {
            let repo = absolute(&args.repo)?;
            let mut tx: Transaction = read_json(&args.file)?;
            let transaction_id = tx.tx_id.clone();
            let snapshot_id = tx.thread.snapshot.index_snapshot.clone();
            let relevant = tx
                .edit
                .operations
                .first()
                .and_then(|operation| {
                    operation
                        .target
                        .get("anchorId")
                        .or_else(|| operation.target.get("ownerSymbolId"))
                })
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned();
            transaction::commit(&repo, &mut tx, &args.target_ref, w).map_err(|error| {
                error
                    .with_transaction(transaction_id)
                    .with_snapshot(snapshot_id)
                    .with_relevant(relevant)
            })
        }),
        #[cfg(any())]
        Command::Tx {
            command: TxCommand::Inspect(args),
        } => transaction::ledger(&absolute(&args.repo)?)?.inspect(&args.transaction_id),
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
    use std::process::Stdio;

    let state = clew::state::StateAuthority::process_default()?;
    let root = state.run_root(run_id)?;
    let mut options = std::fs::OpenOptions::new();
    options.create(true).append(true).write(true);
    #[cfg(unix)]
    options.mode(0o600);
    let stdout = options
        .open(root.join("stdout.log"))
        .map_err(|error| ClewError::new(ErrorCode::Internal, error.to_string()))?;
    let stderr = options
        .open(root.join("stderr.log"))
        .map_err(|error| ClewError::new(ErrorCode::Internal, error.to_string()))?;
    let executable = std::env::current_exe()
        .map_err(|error| ClewError::new(ErrorCode::Internal, error.to_string()))?;
    let mut command = std::process::Command::new(executable);
    command
        .args(["__task-run-execute", "--run", run_id])
        .stdin(Stdio::null())
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr));
    #[cfg(unix)]
    command.process_group(0);
    let child = command
        .spawn()
        .map_err(|error| ClewError::new(ErrorCode::Internal, error.to_string()))?;
    let mut latest = RunRecord::load(run_id)?;
    if latest.status == RunStatus::Created {
        latest.process_id = Some(child.id());
        latest.process_start_token = process_start_token(child.id())?;
        latest.save()?;
    }
    Ok(())
}

fn task_run_status(run_id: &str) -> Result<Value, ClewError> {
    let record = RunRecord::load(run_id)?;
    serde_json::to_value(json!({
        "schema":"codeclew-task-run-status/1.0",
        "run":record,
    }))
    .map_err(parse_error)
}

fn cancel_task_run(run_id: &str) -> Result<Value, ClewError> {
    let mut record = RunRecord::load(run_id)?;
    if matches!(
        record.status,
        RunStatus::ReadyToPublish
            | RunStatus::ValidatedConditional
            | RunStatus::Published
            | RunStatus::WorktreeRecoveryRequired
    ) {
        return Err(ClewError::new(
            ErrorCode::PreconditionFailed,
            "validated or published run cannot be cancelled; retain or recover its candidate",
        ));
    }
    if record.status == RunStatus::Cancelled {
        return task_run_status(run_id);
    }
    let process = record.process_id.zip(record.process_start_token.clone());
    record.status = RunStatus::Cancelled;
    record.failure = None;
    record.save()?;
    if let Some((pid, expected_start)) = process {
        terminate_verified_process_group(pid, &expected_start)?;
    }
    let mut latest = RunRecord::load(run_id)?;
    latest.status = RunStatus::Cancelled;
    latest.process_id = None;
    latest.process_start_token = None;
    latest.save()?;
    task_run_status(run_id)
}

fn process_start_token(pid: u32) -> Result<Option<String>, ClewError> {
    let output = std::process::Command::new("ps")
        .args(["-o", "lstart=", "-p", &pid.to_string()])
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
        .map_err(|error| ClewError::new(ErrorCode::Internal, error.to_string()))?;
    if !output.status.success() {
        return Ok(None);
    }
    let token = String::from_utf8(output.stdout)
        .map_err(|_| ClewError::new(ErrorCode::Internal, "process identity is not UTF-8"))?
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
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
        .map_err(|error| ClewError::new(ErrorCode::Internal, error.to_string()))?;
    if !output.status.success() {
        return Ok(false);
    }
    let status = String::from_utf8(output.stdout)
        .map_err(|_| ClewError::new(ErrorCode::Internal, "process status is not UTF-8"))?;
    Ok(!status.trim().is_empty() && !status.trim_start().starts_with('Z'))
}

#[cfg(unix)]
fn terminate_verified_process_group(pid: u32, expected_start: &str) -> Result<(), ClewError> {
    use std::time::Duration;

    let pid = i32::try_from(pid)
        .ok()
        .filter(|pid| *pid > 1)
        .ok_or_else(|| ClewError::new(ErrorCode::InvalidInput, "run process id is invalid"))?;
    if !process_is_active(pid as u32, expected_start)? {
        return Err(ClewError::new(
            ErrorCode::PreconditionFailed,
            "run process identity changed before cancellation",
        ));
    }
    let signal = |value| {
        let result = unsafe { libc::kill(-pid, value) };
        if result == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH) {
            Ok(())
        } else {
            Err(ClewError::new(
                ErrorCode::Internal,
                std::io::Error::last_os_error().to_string(),
            ))
        }
    };
    signal(libc::SIGTERM)?;
    for _ in 0..40 {
        if !process_is_active(pid as u32, expected_start)? {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    signal(libc::SIGKILL)
}

#[cfg(not(unix))]
fn terminate_verified_process_group(_pid: u32, _expected_start: &str) -> Result<(), ClewError> {
    Err(ClewError::new(
        ErrorCode::UnsupportedProjectConfiguration,
        "run cancellation requires Unix process-group authority",
    ))
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
        record.failure = Some(json!({
            "code":"WORKTREE_RECOVERY_REQUIRED",
            "message":"candidate exists but preparation did not reach a terminal validated state"
        }));
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

fn execute_task_run(workspace: &Path, run_id: &str) -> Result<Value, ClewError> {
    let mut record = RunRecord::load(run_id)?;
    if matches!(
        record.status,
        RunStatus::ReadyToPublish | RunStatus::ValidatedConditional | RunStatus::Published
    ) {
        return task_run_status(run_id);
    }
    record.status = RunStatus::Preparing;
    record.process_id = Some(std::process::id());
    record.process_start_token = process_start_token(std::process::id())?;
    record.failure = None;
    record.save()?;
    let result = prepare_task_run(workspace, &mut record);
    match result {
        Ok(value) => Ok(value),
        Err(error) => {
            if RunRecord::load(run_id)?.status == RunStatus::Cancelled {
                record.status = RunStatus::Cancelled;
            } else {
                record.status = if error.code == ErrorCode::WorktreeRecoveryRequired {
                    RunStatus::WorktreeRecoveryRequired
                } else {
                    RunStatus::Failed
                };
            }
            record.failure = serde_json::to_value(&error).ok();
            record.process_id = None;
            record.process_start_token = None;
            let _ = record.save();
            Err(error)
        }
    }
}

fn prepare_task_run(workspace: &Path, record: &mut RunRecord) -> Result<Value, ClewError> {
    let (session, _) = SessionAuthority::load(&record.session_id)?;
    let repo = session.repository_path()?;
    let current_head = git_head(&repo)?;
    if current_head != session.base_revision {
        return Err(ClewError::new(
            ErrorCode::StaleRequiresReslice,
            "repository HEAD moved after session open",
        ));
    }
    let context_object = session.load_context(&record.context_id)?;
    let plan_object = session.load_plan(&record.plan_id)?;
    if plan_object.context_id != record.context_id {
        return Err(ClewError::new(
            ErrorCode::PreconditionFailed,
            "run context and plan binding differ",
        ));
    }
    let evidence = context_object.evidence;
    let context = &evidence["context"];
    let context_status = context
        .pointer("/completeness/status")
        .and_then(Value::as_str);
    let stdout_status = evidence
        .pointer("/stdoutCompleteness/status")
        .and_then(Value::as_str);
    if context_status != Some("COMPLETE_TASK") || stdout_status != Some("COMPLETE_TASK") {
        return Err(ClewError::new(
            ErrorCode::IncompleteSemanticAnalysis,
            "context must be COMPLETE_TASK before a run can start",
        ));
    }
    let required_threads: Vec<ThreadIr> = serde_json::from_value(
        evidence["threads"]
            .as_array()
            .cloned()
            .map(Value::Array)
            .ok_or_else(|| ClewError::new(ErrorCode::InvalidInput, "context has no threads"))?,
    )
    .map_err(parse_error)?;
    let thread = required_threads.first().cloned().ok_or_else(|| {
        ClewError::new(ErrorCode::InvalidInput, "context has no primary Thread IR")
    })?;
    let mut plan = plan_object.plan;
    clew::task_plan::expand_transient_transform(&mut plan, context, &evidence)?;
    normalize_task_plan(&mut plan)?;
    expand_task_targets(&mut plan, context)?;
    inject_created_type_imports(&mut plan)?;
    inject_explicit_target_imports(&mut plan, context)?;
    inject_created_contract_overrides(&mut plan)?;
    let operations: Vec<EditOperation> = serde_json::from_value(
        plan["operations"]
            .as_array()
            .cloned()
            .map(Value::Array)
            .ok_or_else(|| {
                ClewError::new(ErrorCode::InvalidInput, "plan has no operations array")
            })?,
    )
    .map_err(parse_error)?;
    let expected_write_set: Vec<ExpectedWriteFact> = plan
        .get("expectedWriteSet")
        .cloned()
        .map(serde_json::from_value)
        .transpose()
        .map_err(parse_error)?
        .unwrap_or_default();
    let base_revision = context
        .pointer("/snapshot/baseRevision")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    if base_revision != session.base_revision || base_revision != thread.snapshot.base_revision {
        return Err(ClewError::new(
            ErrorCode::PreconditionFailed,
            "context snapshot does not match session authority",
        ));
    }
    if required_threads.iter().any(|required| {
        required.snapshot.base_revision != base_revision
            || required.snapshot.project_model_hash != thread.snapshot.project_model_hash
            || required.snapshot.compilation != thread.snapshot.compilation
    }) {
        return Err(ClewError::new(
            ErrorCode::PreconditionFailed,
            "context threads do not share one authority",
        ));
    }
    let edit = EditIr {
        schema: "semantic-edit/0.1".into(),
        thread_id: thread.thread_id.clone(),
        base_revision: base_revision.clone(),
        operations,
        expected_write_set,
    };
    let mut test_tasks = context
        .pointer("/validationPlan/targetedArgs")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect::<Vec<_>>();
    include_created_tests(
        &mut test_tasks,
        &plan,
        context
            .pointer("/validationPlan/buildSystem")
            .and_then(Value::as_str)
            .unwrap_or("GRADLE"),
    )?;
    let obligations = context
        .get("verificationObligations")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut transaction = Transaction {
        schema: "semantic-transaction/0.1".into(),
        tx_id: record.transaction_id.clone(),
        actor_id: "codeclew-task-run".into(),
        intent: context
            .pointer("/task/intent")
            .and_then(Value::as_str)
            .unwrap_or("task edit")
            .to_owned(),
        base_revision: base_revision.clone(),
        project_model_hash: thread.snapshot.project_model_hash.clone(),
        base_index_snapshot: Some(thread.snapshot.index_snapshot.clone()),
        status: "CREATED".into(),
        thread,
        required_threads,
        edit,
        preview: None,
        expected_write_set_hash: None,
        actual_write_set_hash: None,
        validation_evidence: vec![json!({
            "kind":"MANAGED_CONTEXT",
            "contextId":record.context_id,
            "contextDigest":canonical::hash(context).map_err(parse_error)?,
            "planId":record.plan_id,
            "runtimeKey":session.runtime_key,
            "runtimeMode":session.runtime_mode,
        })],
        test_tasks,
        candidate_commit: None,
        final_commit: None,
        target_ref: Some(session.target_ref.clone()),
    };
    transaction::ledger(&repo)?.append(&transaction, "run ledger CREATED before prepare")?;
    if !obligations.is_empty() {
        transaction.validation_evidence.push(json!({
            "kind":"UNRESOLVED_VERIFICATION_OBLIGATIONS",
            "obligationsHash":canonical::hash(&obligations).map_err(parse_error)?,
            "publicationBlocking":true,
        }));
    }
    let candidate_root = record.candidate_root()?;
    let repository_state = clew::state::StateAuthority::process_default()?.repository(&repo)?;
    let preparation = with_worker(
        workspace,
        Some(&repository_state.compiler_index),
        |worker| {
            transaction::prepare_candidate(
                &repo,
                &mut transaction,
                &session.target_ref,
                worker,
                &candidate_root,
                !obligations.is_empty(),
            )
        },
    )?;
    let state = clew::state::StateAuthority::process_default()?;
    let run_root = state.run_root(&record.run_id)?;
    state.write_private_atomic(
        &run_root.join("transaction.json"),
        &canonical::bytes(&transaction).map_err(parse_error)?,
    )?;
    record.candidate_commit = transaction.candidate_commit.clone();
    let candidate_worktree = candidate_root.join("worktree");
    let store = clew::cas::CasStore::open(&state)?;
    let (_, candidate_snapshot) = clew::repository_snapshot::capture(&candidate_worktree, &store)?;
    record.candidate_snapshot = Some(candidate_snapshot);
    record.publication_blocked = !obligations.is_empty();
    record.status = if RunRecord::load(&record.run_id)?.status == RunStatus::Cancelled {
        RunStatus::Cancelled
    } else if record.publication_blocked {
        RunStatus::ValidatedConditional
    } else {
        RunStatus::ReadyToPublish
    };
    record.process_id = None;
    record.process_start_token = None;
    record.save()?;
    Ok(json!({
        "schema":"codeclew-task-run-preparation/1.0",
        "run":record,
        "preparation":preparation,
    }))
}

fn publish_task_run(workspace: &Path, session_id: &str, run_id: &str) -> Result<Value, ClewError> {
    let (session, _) = SessionAuthority::load(session_id)?;
    let mut record = RunRecord::load(run_id)?;
    if record.session_id != session_id {
        return Err(ClewError::new(
            ErrorCode::PreconditionFailed,
            "run belongs to another session",
        ));
    }
    if record.publication_blocked || record.status == RunStatus::ValidatedConditional {
        return Err(ClewError::new(
            ErrorCode::IncompleteSemanticAnalysis,
            "conditional run cannot be published; create a new context, plan, and run after discharging obligations",
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
    let repo = session.repository_path()?;
    let state = clew::state::StateAuthority::process_default()?;
    let run_root = state.run_root(run_id)?;
    let mut transaction: Transaction = read_json(&run_root.join("transaction.json"))?;
    let candidate_root = record.candidate_root()?;
    let expected_snapshot = record.candidate_snapshot.as_ref().ok_or_else(|| {
        ClewError::new(
            ErrorCode::TransactionRecoveryRequired,
            "validated run has no immutable candidate snapshot",
        )
    })?;
    let store = clew::cas::CasStore::open(&state)?;
    let (_, observed_snapshot) =
        clew::repository_snapshot::capture(&candidate_root.join("worktree"), &store)?;
    if &observed_snapshot != expected_snapshot {
        return Err(ClewError::new(
            ErrorCode::InputMutated,
            "candidate changed after validation",
        ));
    }
    record.status = RunStatus::Publishing;
    record.save()?;
    let repository_state = state.repository(&repo)?;
    let publication = with_worker(
        workspace,
        Some(&repository_state.compiler_index),
        |worker| {
            transaction::publish_prepared(
                &repo,
                &mut transaction,
                &session.target_ref,
                &candidate_root.join("worktree"),
                worker,
            )
        },
    );
    match publication {
        Ok(value) => {
            state.write_private_atomic(
                &run_root.join("transaction.json"),
                &canonical::bytes(&transaction).map_err(parse_error)?,
            )?;
            record.status = RunStatus::Published;
            record.final_commit = transaction.final_commit.clone();
            record.process_id = None;
            record.process_start_token = None;
            record.save()?;
            Ok(json!({
                "schema":"codeclew-session-publish-result/1.0",
                "run":record,
                "publication":value,
            }))
        }
        Err(error) => {
            record.status = if error.code == ErrorCode::WorktreeRecoveryRequired {
                RunStatus::WorktreeRecoveryRequired
            } else {
                RunStatus::ReadyToPublish
            };
            record.failure = serde_json::to_value(&error).ok();
            record.process_id = None;
            record.process_start_token = None;
            record.save()?;
            Err(error)
        }
    }
}

fn recover_task_run(workspace: &Path, session_id: &str, run_id: &str) -> Result<Value, ClewError> {
    let (session, _) = SessionAuthority::load(session_id)?;
    let mut record = RunRecord::load(run_id)?;
    if record.session_id != session_id {
        return Err(ClewError::new(
            ErrorCode::PreconditionFailed,
            "run belongs to another session",
        ));
    }
    if record.status == RunStatus::Published {
        return task_run_status(run_id);
    }
    if !matches!(
        record.status,
        RunStatus::Publishing | RunStatus::WorktreeRecoveryRequired
    ) {
        return Err(ClewError::new(
            ErrorCode::PreconditionFailed,
            "only a PUBLISHING or WORKTREE_RECOVERY_REQUIRED run can be recovered",
        ));
    }

    let repo = session.repository_path()?;
    let state = clew::state::StateAuthority::process_default()?;
    let run_root = state.run_root(run_id)?;
    let mut transaction: Transaction = read_json(&run_root.join("transaction.json"))?;
    let candidate = transaction.candidate_commit.clone().ok_or_else(|| {
        ClewError::new(
            ErrorCode::TransactionRecoveryRequired,
            "recovery requires a prepared candidate commit",
        )
    })?;
    let current = git_ref_oid(&repo, &session.target_ref)?;

    if current == session.base_revision {
        transaction.status = "READY_TO_PUBLISH".into();
        state.write_private_atomic(
            &run_root.join("transaction.json"),
            &canonical::bytes(&transaction).map_err(parse_error)?,
        )?;
        record.status = RunStatus::ReadyToPublish;
        record.failure = None;
        record.process_id = None;
        record.process_start_token = None;
        record.save()?;
        return publish_task_run(workspace, session_id, run_id);
    }
    if current != candidate {
        return Err(ClewError::new(
            ErrorCode::RefCompareAndSwapFailed,
            "target ref is neither the session base nor the prepared candidate",
        ));
    }

    let candidate_root = record.candidate_root()?;
    let repository_state = state.repository(&repo)?;
    let recovery = with_worker(
        workspace,
        Some(&repository_state.compiler_index),
        |worker| {
            transaction::recover_published_candidate(
                &repo,
                &mut transaction,
                &session.target_ref,
                &candidate_root.join("worktree"),
                worker,
            )
        },
    );
    match recovery {
        Ok(value) => {
            state.write_private_atomic(
                &run_root.join("transaction.json"),
                &canonical::bytes(&transaction).map_err(parse_error)?,
            )?;
            record.status = RunStatus::Published;
            record.final_commit = transaction.final_commit.clone();
            record.failure = None;
            record.process_id = None;
            record.process_start_token = None;
            record.save()?;
            Ok(json!({
                "schema":"codeclew-session-recover-result/1.0",
                "run":record,
                "recovery":value,
            }))
        }
        Err(error) => {
            record.status = RunStatus::WorktreeRecoveryRequired;
            record.failure = serde_json::to_value(&error).ok();
            record.process_id = None;
            record.process_start_token = None;
            record.save()?;
            Err(error)
        }
    }
}

fn git_ref_oid(repo: &Path, reference: &str) -> Result<String, ClewError> {
    let output = std::process::Command::new("git")
        .args(["rev-parse", reference])
        .current_dir(repo)
        .output()
        .map_err(|error| ClewError::new(ErrorCode::Internal, error.to_string()))?;
    if !output.status.success() {
        return Err(ClewError::new(
            ErrorCode::InvalidInput,
            "unable to resolve the session target ref",
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

fn slice_command(worker: &mut WorkerClient, args: SliceArgs) -> Result<Value, ClewError> {
    let output = args.output.clone();
    let thread = build_thread(worker, args)?;
    write_optional(output.as_deref(), &thread)?;
    serde_json::to_value(thread).map_err(parse_error)
}

fn projection_command(worker: &mut WorkerClient, args: ProjectionArgs) -> Result<Value, ClewError> {
    let output = args.output.clone();
    let level = projection_level(args.level);
    let thread_kind = projection_thread_kind(args.thread_kind);
    let thread = build_thread(
        worker,
        SliceArgs {
            repo: args.repo,
            compilation: args.compilation,
            symbol: args.symbol,
            file: args.file,
            offset: args.offset,
            direction: args.direction,
            max_nodes: args.max_nodes,
            output: None,
        },
    )?;
    let adapted = thread_projection::from_thread(&thread, thread_kind, args.claim.as_deref())
        .map_err(|error| {
            ClewError::new(ErrorCode::IncompleteSemanticAnalysis, error.to_string())
        })?;
    let query = ProjectionQuery {
        schema: projection::PROJECTION_SCHEMA.into(),
        level,
        roots: vec![adapted.root_fact_id],
        thread_kinds: vec![thread_kind],
        traversal: Traversal::Both,
        budget: ProjectionBudget {
            max_nodes: args.max_nodes,
            max_bytes: args.max_bytes,
        },
        boundary_policy: if args.refuse_on_boundary {
            BoundaryPolicy::Refuse
        } else {
            BoundaryPolicy::ReturnPartial
        },
    };
    let result = projection::project(&adapted.input, &query).map_err(|error| {
        let code = if error == projection::ProjectionError::BudgetTooSmall {
            ErrorCode::SliceBudgetExceeded
        } else {
            ErrorCode::IncompleteSemanticAnalysis
        };
        ClewError::new(code, error.to_string())
    })?;
    projection::validate_projection(&adapted.input, &query, &result).map_err(|error| {
        ClewError::new(ErrorCode::IncompleteSemanticAnalysis, error.to_string())
    })?;
    write_optional(output.as_deref(), &result)?;
    serde_json::to_value(result).map_err(parse_error)
}

fn projection_level(level: ProjectionLevelArg) -> ProjectionLevel {
    match level {
        ProjectionLevelArg::L0 => ProjectionLevel::L0,
        ProjectionLevelArg::L1 => ProjectionLevel::L1,
        ProjectionLevelArg::L2 => ProjectionLevel::L2,
        ProjectionLevelArg::L3 => ProjectionLevel::L3,
        ProjectionLevelArg::L4 => ProjectionLevel::L4,
        ProjectionLevelArg::L5 => ProjectionLevel::L5,
    }
}

fn projection_thread_kind(kind: ProjectionThreadKindArg) -> ThreadKind {
    match kind {
        ProjectionThreadKindArg::Control => ThreadKind::Control,
        ProjectionThreadKindArg::Data => ThreadKind::Data,
        ProjectionThreadKindArg::Journey => ThreadKind::Journey,
        ProjectionThreadKindArg::State => ThreadKind::State,
        ProjectionThreadKindArg::Effect => ThreadKind::Effect,
        ProjectionThreadKindArg::Failure => ThreadKind::Failure,
        ProjectionThreadKindArg::Config => ThreadKind::Config,
        ProjectionThreadKindArg::TestEvidence => ThreadKind::TestEvidence,
        ProjectionThreadKindArg::Change => ThreadKind::Change,
    }
}

fn build_task_thread(
    worker: &mut WorkerClient,
    repo: &Path,
    compilation: &str,
    project: &Value,
    index_snapshot: &str,
    resolution: &Value,
) -> Result<ThreadIr, ClewError> {
    let symbol = resolution
        .pointer("/declaration/legacySymbolId")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            ClewError::new(
                ErrorCode::WorkerProtocolMismatch,
                "resolved task root has no legacySymbolId",
            )
        })?;
    let raw = worker.request(
        RequestKind::BuildLocalGraph,
        &json!({"repo":repo,"symbol":symbol,"compilation":compilation}),
    )?;
    let graph: LocalGraph = serde_json::from_value(raw).map_err(parse_error)?;
    let graph = graph::enrich(graph);
    let seed_id = graph
        .nodes
        .iter()
        .filter(|node| node.kind == "RETURN")
        .max_by_key(|node| {
            node.origin
                .as_ref()
                .and_then(|origin| origin.pointer("/rangeHint/1"))
                .and_then(Value::as_u64)
                .unwrap_or_default()
        })
        .or_else(|| {
            graph
                .nodes
                .iter()
                .filter(|node| node.origin.is_some())
                .max_by_key(|node| {
                    node.origin
                        .as_ref()
                        .and_then(|origin| origin.pointer("/rangeHint/1"))
                        .and_then(Value::as_u64)
                        .unwrap_or_default()
                })
        })
        .map(|node| node.id.clone())
        .ok_or_else(|| {
            ClewError::new(
                ErrorCode::IncompleteSemanticAnalysis,
                format!("task root {symbol} has no source-backed graph seed"),
            )
        })?;
    let seed_node = graph.nodes.iter().find(|node| node.id == seed_id);
    let snapshot = Snapshot {
        base_revision: git_head(repo)?,
        project_model_hash: project["projectModelHash"]
            .as_str()
            .unwrap_or_default()
            .into(),
        compiler_version: worker.capabilities.compiler_version.clone(),
        build_system: match project["buildSystem"].as_str() {
            Some("MAVEN") => BuildSystem::Maven,
            _ => BuildSystem::Gradle,
        },
        build_launcher: project["buildLauncher"]
            .as_str()
            .unwrap_or("./gradlew")
            .into(),
        index_snapshot: index_snapshot.into(),
        compilation: compilation.into(),
        compile_task: project["compileTask"]
            .as_str()
            .unwrap_or(":compileKotlin")
            .into(),
        test_tasks: project["testTasks"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect(),
    };
    let seed = json!({
        "kind":"TASK_ROOT",
        "symbol":symbol,
        "nodeId":seed_id,
        "anchor":seed_node.and_then(|node|node.origin.clone())
    });
    graph::slice(&graph, &seed_id, SlicePolicy::default(), snapshot, seed)
        .map_err(|error| ClewError::new(ErrorCode::Internal, error.to_string()))
}

fn build_thread(worker: &mut WorkerClient, args: SliceArgs) -> Result<ThreadIr, ClewError> {
    let repo = absolute(&args.repo)?;
    let (project, verified_index_facts) = worker
        .open_project_and_index_verified(&json!({"repo":repo,"compilation":args.compilation}))?;
    let mut repository_index = RepositoryIndex::open_compilation(&repo, Some(&args.compilation))?;
    let index_snapshot = repository_index.update_verified(&verified_index_facts, worker)?;
    repository_index.require_fresh(REPOSITORY_INDEX_FACT)?;
    let expression_anchor = if let (Some(file), Some(offset)) = (&args.file, args.offset) {
        Some(
            worker.request(
                RequestKind::ResolveExpression,
                &json!({"repo":repo,"file":file,"offset":offset,"compilation":args.compilation}),
            )?["anchor"]
                .clone(),
        )
    } else {
        None
    };
    let symbol = args
        .symbol
        .clone()
        .or_else(|| {
            expression_anchor
                .as_ref()?
                .get("ownerSymbolId")?
                .as_str()
                .map(str::to_owned)
        })
        .ok_or_else(|| {
            ClewError::new(
                ErrorCode::InvalidInput,
                "slice needs --symbol or --file with --offset",
            )
        })?;
    let raw = worker.request(
        RequestKind::BuildLocalGraph,
        &json!({"repo":repo,"symbol":symbol,"compilation":args.compilation}),
    )?;
    let graph: LocalGraph = serde_json::from_value(raw).map_err(parse_error)?;
    let graph = graph::enrich(graph);
    let seed_id = if let Some(anchor) = &expression_anchor {
        let anchor_id = anchor.get("anchorId").and_then(Value::as_str);
        graph
            .nodes
            .iter()
            .find(|node| {
                node.origin
                    .as_ref()
                    .and_then(|o| o.get("anchorId"))
                    .and_then(Value::as_str)
                    == anchor_id
            })
            .map(|n| n.id.clone())
            .ok_or_else(|| {
                ClewError::new(
                    ErrorCode::ExpressionNotFound,
                    "resolved expression has no source-backed CFG node",
                )
            })?
    } else {
        graph
            .nodes
            .iter()
            .filter(|n| n.kind == "RETURN")
            .max_by_key(|n| {
                n.origin
                    .as_ref()
                    .and_then(|o| o.pointer("/rangeHint/0"))
                    .and_then(Value::as_u64)
                    .unwrap_or(0)
            })
            .map(|n| n.id.clone())
            .or_else(|| {
                graph
                    .nodes
                    .iter()
                    .filter(|n| n.origin.is_some())
                    .max_by_key(|n| {
                        n.origin
                            .as_ref()
                            .and_then(|o| o.pointer("/rangeHint/1"))
                            .and_then(Value::as_u64)
                            .unwrap_or(0)
                    })
                    .map(|n| n.id.clone())
            })
            .ok_or_else(|| {
                ClewError::new(
                    ErrorCode::IncompleteSemanticAnalysis,
                    "function has no source-backed CFG seed",
                )
            })?
    };
    let seed_node = graph.nodes.iter().find(|n| n.id == seed_id);
    let seed = json!({"kind":if expression_anchor.is_some(){"EXPRESSION"}else{"FUNCTION_RETURN"},"symbol":symbol,"nodeId":seed_id,"anchor":expression_anchor.or_else(||seed_node.and_then(|n|n.origin.clone()))});
    let policy = SlicePolicy {
        direction: match args.direction {
            DirectionArg::Forward => Direction::Forward,
            DirectionArg::Backward => Direction::Backward,
            DirectionArg::Both => Direction::Both,
        },
        max_nodes: args.max_nodes,
        ..Default::default()
    };
    let snapshot = Snapshot {
        base_revision: git_head(&repo)?,
        project_model_hash: project["projectModelHash"]
            .as_str()
            .unwrap_or_default()
            .into(),
        compiler_version: worker.capabilities.compiler_version.clone(),
        build_system: match project["buildSystem"].as_str() {
            Some("MAVEN") => BuildSystem::Maven,
            _ => BuildSystem::Gradle,
        },
        build_launcher: project["buildLauncher"]
            .as_str()
            .unwrap_or("./gradlew")
            .into(),
        index_snapshot,
        compilation: args.compilation,
        compile_task: project["compileTask"]
            .as_str()
            .unwrap_or(":compileKotlin")
            .into(),
        test_tasks: project["testTasks"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect(),
    };
    let thread = graph::slice(&graph, &seed_id, policy, snapshot, seed)
        .map_err(|e| ClewError::new(ErrorCode::Internal, e.to_string()))?;
    Ok(thread)
}

fn start_cli_worker(
    workspace: &Path,
    compiler_index_root: Option<&Path>,
) -> Result<WorkerClient, ClewError> {
    match compiler_index_root {
        Some(root) => {
            let inherited_build_state = inherited_build_state_root();
            WorkerClient::start_with_states(workspace, inherited_build_state.as_deref(), Some(root))
        }
        None => WorkerClient::start(workspace),
    }
}

fn with_worker<F, T>(
    workspace: &Path,
    compiler_index_root: Option<&Path>,
    action: F,
) -> Result<T, ClewError>
where
    F: FnOnce(&mut WorkerClient) -> Result<T, ClewError>,
{
    let mut worker = start_cli_worker(workspace, compiler_index_root)?;
    let result = action(&mut worker);
    let shutdown = worker.shutdown();
    match (result, shutdown) {
        (Ok(v), Ok(())) => Ok(v),
        (Err(e), _) => Err(e),
        (_, Err(e)) => Err(e),
    }
}
fn absolute(path: &Path) -> Result<PathBuf, ClewError> {
    path.canonicalize()
        .map_err(|e| ClewError::new(ErrorCode::InvalidInput, format!("{}: {e}", path.display())))
}

fn git_head(repo: &Path) -> Result<String, ClewError> {
    let output = std::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(repo)
        .output()
        .map_err(|e| ClewError::new(ErrorCode::InvalidInput, e.to_string()))?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).trim().into())
    } else {
        Err(ClewError::new(
            ErrorCode::InvalidInput,
            "repository must have a committed Git HEAD",
        ))
    }
}
fn read_json<T: DeserializeOwned>(path: &Path) -> Result<T, ClewError> {
    let bytes =
        std::fs::read(path).map_err(|e| ClewError::new(ErrorCode::InvalidInput, e.to_string()))?;
    serde_json::from_slice(&bytes).map_err(parse_error)
}
fn read_typed_goal_request(args: &TypedGoalArgs) -> Result<TypedGoalBindingRequest, ClewError> {
    let (bytes, require_canonical) = match (&args.request, &args.request_json) {
        (Some(path), None) => {
            let metadata = std::fs::metadata(path)
                .map_err(|error| ClewError::new(ErrorCode::InvalidInput, error.to_string()))?;
            if metadata.len() > TYPED_GOAL_MAX_REQUEST_BYTES as u64 {
                return Err(ClewError::new(
                    ErrorCode::InvalidInput,
                    "typed-goal request exceeds 16 KiB",
                ));
            }
            let bytes = std::fs::read(path)
                .map_err(|error| ClewError::new(ErrorCode::InvalidInput, error.to_string()))?;
            (bytes, false)
        }
        (None, Some(inline)) => (inline.as_bytes().to_vec(), true),
        _ => {
            return Err(ClewError::new(
                ErrorCode::InvalidInput,
                "exactly one typed-goal request transport is required",
            ));
        }
    };
    if bytes.len() > TYPED_GOAL_MAX_REQUEST_BYTES {
        return Err(ClewError::new(
            ErrorCode::InvalidInput,
            "typed-goal request exceeds 16 KiB",
        ));
    }
    let request: TypedGoalBindingRequest = serde_json::from_slice(&bytes).map_err(parse_error)?;
    if require_canonical
        && canonical::bytes(&request)
            .map_err(|error| ClewError::new(ErrorCode::InvalidInput, error.to_string()))?
            != bytes
    {
        return Err(ClewError::new(
            ErrorCode::InvalidInput,
            "--request-json must use canonical JSON encoding",
        ));
    }
    Ok(request)
}

fn validate_conditional_decision(
    decision: &Value,
    expected_revision: &str,
) -> Result<Vec<Value>, ClewError> {
    let schema = decision.get("schema").and_then(Value::as_str);
    if !matches!(
        schema,
        Some(TYPED_GOAL_BINDING_DECISION_SCHEMA | MAP_EDGE_WITH_CONTEXT_DECISION_SCHEMA)
    ) || decision.get("status").and_then(Value::as_str) != Some("CONDITIONAL")
        || decision.get("revision").and_then(Value::as_str) != Some(expected_revision)
    {
        return Err(ClewError::new(
            ErrorCode::InvalidInput,
            "conditional decision schema, status, or revision is invalid",
        ));
    }
    let valid_digest = |field: &str| {
        decision
            .get(field)
            .and_then(Value::as_str)
            .is_some_and(|value| {
                value.len() == 71
                    && value.starts_with("sha256:")
                    && value[7..]
                        .bytes()
                        .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
            })
    };
    if !valid_digest("goalFingerprint")
        || !(valid_digest("evidenceFingerprint") || valid_digest("establishedEvidenceFingerprint"))
    {
        return Err(ClewError::new(
            ErrorCode::InvalidInput,
            "conditional decision has no canonical evidence identity",
        ));
    }
    let obligations = decision
        .get("unresolvedObligations")
        .and_then(Value::as_array)
        .filter(|obligations| !obligations.is_empty())
        .ok_or_else(|| {
            ClewError::new(
                ErrorCode::InvalidInput,
                "conditional decision has no unresolved obligations",
            )
        })?;
    let mut ids = BTreeSet::new();
    for obligation in obligations {
        let id = obligation
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let code = obligation
            .get("code")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let subject = obligation.get("subject").and_then(Value::as_array);
        if id.is_empty()
            || !ids.insert(id)
            || !matches!(
                code,
                "VERIFY_CALL_TARGET_IDENTITY"
                    | "VERIFY_ARGUMENT_PARAMETER_MAPPING"
                    | "VERIFY_BEHAVIORAL_ORACLE"
            )
            || subject.is_none_or(Vec::is_empty)
            || obligation
                .get("publicationBlocking")
                .and_then(Value::as_bool)
                != Some(true)
        {
            return Err(ClewError::new(
                ErrorCode::InvalidInput,
                "conditional verification obligation is malformed or non-blocking",
            ));
        }
    }
    Ok(obligations.clone())
}
fn typed_goal_refusal_json(reason: TypedGoalRefusalReason) -> Result<Value, ClewError> {
    serde_json::to_value(TypedGoalRefusal {
        schema: TYPED_GOAL_BINDING_DECISION_SCHEMA.into(),
        status: "REFUSED".into(),
        reason,
        rejections: vec![],
        declaration_rejections: vec![],
    })
    .map_err(parse_error)
}
fn write_optional<T: serde::Serialize>(path: Option<&Path>, value: &T) -> Result<(), ClewError> {
    if let Some(path) = path {
        std::fs::write(
            path,
            canonical::pretty(value)
                .map_err(|e| ClewError::new(ErrorCode::Internal, e.to_string()))?,
        )
        .map_err(|e| ClewError::new(ErrorCode::Internal, e.to_string()))?
    }
    Ok(())
}
fn write_artifact<T: serde::Serialize>(path: &Path, value: &T) -> Result<(), ClewError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| ClewError::new(ErrorCode::Internal, e.to_string()))?;
    }
    std::fs::write(
        path,
        canonical::pretty(value).map_err(|e| ClewError::new(ErrorCode::Internal, e.to_string()))?,
    )
    .map_err(|e| ClewError::new(ErrorCode::Internal, e.to_string()))
}

fn expand_task_targets(plan: &mut Value, context: &Value) -> Result<(), ClewError> {
    let mut targets = std::collections::BTreeMap::<String, Value>::new();
    for (key, prefix) in [("editSurfaces", "S"), ("contracts", "C"), ("tests", "T")] {
        for (index, item) in context[key].as_array().into_iter().flatten().enumerate() {
            for target_key in ["declarationTarget", "bodyTarget"] {
                let Some(target) = item.get(target_key) else {
                    continue;
                };
                let Some(id) = target["anchorId"].as_str() else {
                    continue;
                };
                targets.insert(id.to_owned(), target.clone());
                let suffix = if target_key == "bodyTarget" { "B" } else { "" };
                targets.insert(format!("{prefix}{}{suffix}", index + 1), target.clone());
            }
        }
    }
    let mut model_input_targets = std::collections::BTreeMap::<String, Value>::new();
    for (index, item) in context["modelInputSurfaces"]
        .as_array()
        .into_iter()
        .flatten()
        .enumerate()
    {
        let Some(target) = item.get("modelInputTarget") else {
            continue;
        };
        let alias = format!("M{}", index + 1);
        model_input_targets.insert(alias, target.clone());
        if let Some(target_id) = item.get("targetId").and_then(Value::as_str) {
            model_input_targets.insert(target_id.to_owned(), target.clone());
        }
        if let Some(anchor_id) = target.get("anchorId").and_then(Value::as_str) {
            model_input_targets.insert(anchor_id.to_owned(), target.clone());
        }
    }
    for operation in plan["operations"].as_array_mut().into_iter().flatten() {
        if operation["kind"] == "CREATE_FILE" {
            continue;
        }
        let Some(target_id) = operation
            .pointer("/target/targetId")
            .or_else(|| operation.pointer("/target/declarationTargetId"))
            .or_else(|| operation.pointer("/target/bodyTargetId"))
            .and_then(Value::as_str)
            .map(str::to_owned)
        else {
            return Err(ClewError::new(
                ErrorCode::InvalidInput,
                "every non-CREATE_FILE task operation must reference a context targetId",
            ));
        };
        let is_model_input = operation["kind"] == "REPLACE_MODEL_INPUT";
        operation["target"] = if is_model_input {
            model_input_targets.get(&target_id)
        } else {
            targets.get(&target_id)
        }
        .cloned()
        .ok_or_else(|| {
            ClewError::new(
                ErrorCode::InvalidInput,
                if is_model_input {
                    format!("model input edit references unknown emitted M target {target_id}")
                } else {
                    format!("edit plan references unknown declaration target {target_id}")
                },
            )
        })?;
    }
    Ok(())
}
fn normalize_task_plan(plan: &mut Value) -> Result<(), ClewError> {
    if plan.get("operations").is_none()
        && let Some(edits) = plan.as_object_mut().and_then(|plan| plan.remove("edits"))
    {
        plan["operations"] = edits;
    }
    let operations = plan["operations"].as_array_mut().ok_or_else(|| {
        ClewError::new(ErrorCode::InvalidInput, "edit plan has no operations array")
    })?;
    for (index, operation) in operations.iter_mut().enumerate() {
        let object = operation.as_object_mut().ok_or_else(|| {
            ClewError::new(ErrorCode::InvalidInput, "task operation must be an object")
        })?;
        object
            .entry("opId")
            .or_insert_with(|| json!(format!("task-op-{}", index + 1)));
        if !object.contains_key("kind") {
            let kind = object.remove("op").ok_or_else(|| {
                ClewError::new(ErrorCode::InvalidInput, "task operation must contain kind")
            })?;
            object.insert("kind".to_owned(), kind);
        }
        if object.get("kind").and_then(Value::as_str) == Some("REPLACE_MODEL_INPUT") {
            if object.contains_key("path")
                || object.contains_key("replacement")
                || object.contains_key("substitutions")
                || object.contains_key("old")
                || object.contains_key("oldLines")
                || object.contains_key("new")
            {
                return Err(ClewError::new(
                    ErrorCode::InvalidInput,
                    "REPLACE_MODEL_INPUT accepts only an emitted M target and top-level newLines",
                ));
            }
            let lines = object.remove("newLines").ok_or_else(|| {
                ClewError::new(
                    ErrorCode::InvalidInput,
                    "REPLACE_MODEL_INPUT requires a complete newLines array",
                )
            })?;
            let replacement = join_exact_plan_lines(&lines)?;
            object.insert("replacement".to_owned(), json!({"kotlin":replacement}));
            object.entry("preconditions").or_insert_with(|| json!({}));
            object.entry("postconditions").or_insert_with(|| json!({}));
            continue;
        }
        if object.get("kind").and_then(Value::as_str) == Some("CREATE_FILE")
            && !object.contains_key("replacement")
        {
            let lines = object
                .remove("kotlinLines")
                .or_else(|| object.remove("newLines"))
                .ok_or_else(|| {
                    ClewError::new(
                        ErrorCode::InvalidInput,
                        "CREATE_FILE needs replacement.kotlinLines or top-level kotlinLines/newLines",
                    )
                })?;
            let path = object
                .remove("path")
                .and_then(|path| path.as_str().map(str::to_owned))
                .or_else(|| {
                    object
                        .get("target")
                        .and_then(|target| target.get("fileId").or_else(|| target.get("targetId")))
                        .and_then(Value::as_str)
                        .map(|path| path.strip_prefix("file:").unwrap_or(path).to_owned())
                })
                .ok_or_else(|| {
                    ClewError::new(ErrorCode::InvalidInput, "CREATE_FILE needs a path")
                })?;
            object.insert("target".to_owned(), json!({"fileId":path}));
            object.insert("replacement".to_owned(), json!({"kotlinLines":lines}));
        }
        if object.contains_key("old") || object.contains_key("oldLines") {
            let mut substitution = serde_json::Map::new();
            for key in [
                "old",
                "new",
                "oldLines",
                "newLines",
                "occurrence",
                "occurrences",
            ] {
                if let Some(value) = object.remove(key) {
                    substitution.insert(key.to_owned(), value);
                }
            }
            object.insert(
                "substitutions".to_owned(),
                Value::Array(vec![Value::Object(substitution)]),
            );
        }
        if let Some(substitutions) = object.remove("substitutions") {
            if object.contains_key("preconditions") {
                return Err(ClewError::new(
                    ErrorCode::InvalidInput,
                    "task operation cannot contain both substitutions and preconditions",
                ));
            }
            object.insert(
                "preconditions".to_owned(),
                json!({"substitutions":substitutions}),
            );
        }
        if let Some(replacement) = object.get_mut("replacement").and_then(Value::as_object_mut) {
            join_plan_lines(replacement, "kotlinLines", "kotlin")?;
        }
        if let Some(substitutions) = object
            .get_mut("preconditions")
            .and_then(|preconditions| preconditions.get_mut("substitutions"))
            .and_then(Value::as_array_mut)
        {
            for substitution in substitutions {
                let substitution = substitution.as_object_mut().ok_or_else(|| {
                    ClewError::new(
                        ErrorCode::InvalidInput,
                        "task substitution must be an object",
                    )
                })?;
                if join_plan_lines(substitution, "oldLines", "old")? {
                    substitution.insert("lineMode".to_owned(), Value::Bool(true));
                }
                join_plan_lines(substitution, "newLines", "new")?;
            }
        }
        if object.get("kind").and_then(Value::as_str) == Some("REWRITE_DECLARATION") {
            object
                .entry("replacement")
                .or_insert_with(|| json!({"kotlin":""}));
        }
    }
    let unmerged = std::mem::take(operations);
    let mut merged = Vec::<Value>::new();
    for operation in unmerged {
        let merge_target =
            (operation["kind"] == "REWRITE_DECLARATION").then(|| operation["target"].clone());
        if let Some(target) = merge_target
            && let Some(existing) = merged.iter_mut().find(|existing| {
                existing["kind"] == "REWRITE_DECLARATION" && existing["target"] == target
            })
        {
            let additions = operation
                .pointer("/preconditions/substitutions")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            existing
                .pointer_mut("/preconditions/substitutions")
                .and_then(Value::as_array_mut)
                .expect("normalized rewrite substitutions")
                .extend(additions);
            continue;
        }
        merged.push(operation);
    }
    *operations = merged;
    Ok(())
}

fn join_exact_plan_lines(lines: &Value) -> Result<String, ClewError> {
    let lines = lines.as_array().ok_or_else(|| {
        ClewError::new(
            ErrorCode::InvalidInput,
            "REPLACE_MODEL_INPUT newLines must be an array of strings",
        )
    })?;
    let lines = lines
        .iter()
        .map(Value::as_str)
        .collect::<Option<Vec<_>>>()
        .ok_or_else(|| {
            ClewError::new(
                ErrorCode::InvalidInput,
                "REPLACE_MODEL_INPUT newLines must contain only strings",
            )
        })?;
    if lines
        .iter()
        .any(|line| line.contains('\n') || line.contains('\r'))
    {
        return Err(ClewError::new(
            ErrorCode::InvalidInput,
            "REPLACE_MODEL_INPUT newLines items must each contain exactly one line",
        ));
    }
    Ok(lines.join("\n"))
}
fn join_plan_lines(
    object: &mut serde_json::Map<String, Value>,
    lines_key: &str,
    text_key: &str,
) -> Result<bool, ClewError> {
    let Some(lines) = object.remove(lines_key) else {
        return Ok(false);
    };
    if object.contains_key(text_key) {
        return Err(ClewError::new(
            ErrorCode::InvalidInput,
            format!("task edit cannot contain both {text_key} and {lines_key}"),
        ));
    }
    let lines = if let Some(text) = lines.as_str() {
        text.lines().collect::<Vec<_>>()
    } else {
        lines
            .as_array()
            .ok_or_else(|| {
                ClewError::new(
                    ErrorCode::InvalidInput,
                    format!("task edit field {lines_key} must be a string or array of strings"),
                )
            })?
            .iter()
            .map(Value::as_str)
            .collect::<Option<Vec<_>>>()
            .ok_or_else(|| {
                ClewError::new(
                    ErrorCode::InvalidInput,
                    format!("task edit field {lines_key} must contain only strings"),
                )
            })?
    };
    let common_indent = lines
        .iter()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            line.char_indices()
                .find_map(|(index, character)| (!character.is_whitespace()).then_some(index))
                .unwrap_or(line.len())
        })
        .min()
        .unwrap_or_default();
    let text = lines
        .iter()
        .map(|line| {
            if line.trim().is_empty() {
                ""
            } else {
                &line[common_indent..]
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    object.insert(text_key.to_owned(), Value::String(text));
    Ok(true)
}
fn inject_created_contract_overrides(plan: &mut Value) -> Result<(), ClewError> {
    let contracts = plan["operations"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|operation| operation["kind"] == "CREATE_FILE")
        .filter_map(|operation| {
            operation
                .pointer("/replacement/kotlin")
                .and_then(Value::as_str)
        })
        .flat_map(|source| {
            let lines = source.lines().collect::<Vec<_>>();
            let mut contracts = Vec::new();
            for (index, line) in lines.iter().enumerate() {
                let Some(declaration) = line.trim().strip_prefix("interface ") else {
                    continue;
                };
                let name = declaration
                    .split(|character: char| !character.is_ascii_alphanumeric() && character != '_')
                    .next()
                    .unwrap_or_default()
                    .to_owned();
                let fields = lines[index + 1..]
                    .iter()
                    .take_while(|line| !line.trim().starts_with('}'))
                    .filter_map(|line| {
                        line.trim()
                            .strip_prefix("val ")
                            .or_else(|| line.trim().strip_prefix("var "))?
                            .split_once(':')
                            .map(|(field, _)| field.trim().to_owned())
                    })
                    .collect::<Vec<_>>();
                if !name.is_empty() && !fields.is_empty() {
                    contracts.push((name, fields));
                }
            }
            contracts
        })
        .collect::<Vec<_>>();
    for operation in plan["operations"].as_array_mut().into_iter().flatten() {
        if operation["kind"] != "REWRITE_DECLARATION" {
            continue;
        }
        let substitutions = operation
            .pointer_mut("/preconditions/substitutions")
            .and_then(Value::as_array_mut)
            .ok_or_else(|| {
                ClewError::new(
                    ErrorCode::InvalidInput,
                    "normalized rewrite has no substitutions",
                )
            })?;
        let replacement_text = substitutions
            .iter()
            .filter_map(|substitution| substitution["new"].as_str())
            .collect::<Vec<_>>()
            .join("\n");
        for (name, fields) in &contracts {
            if !replacement_text.contains(&format!(") : {name}")) {
                continue;
            }
            for field in fields {
                if replacement_text.contains(&format!("override val {field}:"))
                    || replacement_text.contains(&format!("override var {field}:"))
                {
                    continue;
                }
                substitutions.push(json!({
                    "old":format!("val {field}:"),
                    "new":format!("override val {field}:"),
                    "occurrences":1
                }));
            }
        }
    }
    Ok(())
}
fn inject_created_type_imports(plan: &mut Value) -> Result<(), ClewError> {
    let operations = plan["operations"].as_array().ok_or_else(|| {
        ClewError::new(ErrorCode::InvalidInput, "edit plan has no operations array")
    })?;
    let created_types = operations
        .iter()
        .filter(|operation| operation["kind"] == "CREATE_FILE")
        .flat_map(|operation| {
            let source = operation
                .pointer("/replacement/kotlin")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let package = source
                .lines()
                .find_map(|line| line.trim().strip_prefix("package "))
                .unwrap_or_default();
            source
                .lines()
                .filter(|line| !line.starts_with(char::is_whitespace))
                .filter_map(|line| {
                    let words = line.split_whitespace().collect::<Vec<_>>();
                    let marker = words
                        .iter()
                        .position(|word| matches!(*word, "class" | "interface"))?;
                    let name = words.get(marker + 1)?.trim_matches(|character: char| {
                        !character.is_ascii_alphanumeric() && character != '_'
                    });
                    (!package.is_empty() && !name.is_empty()).then(|| {
                        (
                            name.to_owned(),
                            format!("{package}.{name}"),
                            package.to_owned(),
                        )
                    })
                })
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    let mut expanded = Vec::new();
    let mut inserted = std::collections::BTreeSet::new();
    for operation in operations {
        if operation["kind"] == "REWRITE_DECLARATION" {
            let file = operation
                .pointer("/target/fileId")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let target_package = file
                .split_once("/kotlin/")
                .and_then(|(_, relative)| relative.rsplit_once('/'))
                .map(|(package, _)| package.replace('/', "."))
                .unwrap_or_default();
            let replacements = operation
                .pointer("/preconditions/substitutions")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|substitution| substitution["new"].as_str())
                .collect::<Vec<_>>();
            for (name, fq_name, package) in &created_types {
                let needs_import = replacements.iter().any(|replacement| {
                    contains_identifier(&replacement.replace(fq_name, ""), name)
                });
                if target_package == *package
                    || !needs_import
                    || !inserted.insert((file.to_owned(), fq_name.clone()))
                {
                    continue;
                }
                expanded.push(json!({
                    "opId":format!("auto-import-{}",expanded.len()+1),
                    "kind":"ADD_IMPORT",
                    "target":operation["target"].clone(),
                    "replacement":{"kotlin":fq_name},
                    "preconditions":{},
                    "postconditions":{}
                }));
            }
        }
        expanded.push(operation.clone());
    }
    plan["operations"] = Value::Array(expanded);
    Ok(())
}

fn inject_explicit_target_imports(plan: &mut Value, context: &Value) -> Result<(), ClewError> {
    let operations = plan["operations"].as_array().ok_or_else(|| {
        ClewError::new(ErrorCode::InvalidInput, "edit plan has no operations array")
    })?;
    let explicit_targets = context["editSurfaces"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|surface| surface["role"] == "EXPLICIT_TARGET")
        .filter_map(|surface| {
            let name = surface["name"].as_str()?;
            let identity: Value = serde_json::from_str(
                surface
                    .pointer("/declarationTarget/ownerSymbolId")?
                    .as_str()?,
            )
            .ok()?;
            let package = identity["package"].as_str()?;
            let top_level = identity["containingDeclarations"]
                .as_array()
                .is_some_and(Vec::is_empty);
            let is_extension = identity["receiverTypes"]
                .as_array()
                .is_some_and(|receivers| !receivers.is_empty());
            (!name.is_empty() && !package.is_empty() && top_level).then(|| {
                (
                    name.to_owned(),
                    format!("{package}.{name}"),
                    package.to_owned(),
                    is_extension,
                )
            })
        })
        .collect::<Vec<_>>();
    let mut expanded = Vec::new();
    let mut inserted = std::collections::BTreeSet::new();
    for original in operations {
        let mut operation = original.clone();
        if operation["kind"] == "REWRITE_DECLARATION" {
            let file = operation
                .pointer("/target/fileId")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned();
            let target_package = file
                .split_once("/kotlin/")
                .and_then(|(_, relative)| relative.rsplit_once('/'))
                .map(|(package, _)| package.replace('/', "."))
                .unwrap_or_default();
            for (name, fq_name, package, is_extension) in &explicit_targets {
                let mut needs_import = false;
                for substitution in operation
                    .pointer_mut("/preconditions/substitutions")
                    .and_then(Value::as_array_mut)
                    .into_iter()
                    .flatten()
                {
                    let Some(replacement) = substitution["new"].as_str() else {
                        continue;
                    };
                    needs_import |=
                        replacement.contains(fq_name) || contains_identifier(replacement, name);
                    if replacement.contains(fq_name) {
                        substitution["new"] = json!(if *is_extension {
                            canonicalize_extension_calls(replacement, fq_name, name)
                        } else {
                            replacement.replace(fq_name, name)
                        });
                    }
                }
                if target_package == *package
                    || !needs_import
                    || !inserted.insert((file.clone(), fq_name.clone()))
                {
                    continue;
                }
                expanded.push(json!({
                    "opId":format!("auto-explicit-import-{}",expanded.len()+1),
                    "kind":"ADD_IMPORT",
                    "target":operation["target"].clone(),
                    "replacement":{"kotlin":fq_name},
                    "preconditions":{},
                    "postconditions":{}
                }));
            }
        }
        expanded.push(operation);
    }
    plan["operations"] = Value::Array(expanded);
    Ok(())
}

fn canonicalize_extension_calls(source: &str, fq_name: &str, name: &str) -> String {
    let needle = format!("{fq_name}(");
    let mut result = source.to_owned();
    let mut cursor = 0usize;
    while let Some(relative) = result[cursor..].find(&needle) {
        let start = cursor + relative;
        let arguments_start = start + needle.len();
        let mut depth = 0isize;
        let mut comma = None;
        for (offset, character) in result[arguments_start..].char_indices() {
            match character {
                '(' | '[' | '{' => depth += 1,
                ')' | ']' | '}' if depth > 0 => depth -= 1,
                ')' if depth == 0 => break,
                ',' if depth == 0 => {
                    comma = Some(arguments_start + offset);
                    break;
                }
                _ => {}
            }
        }
        let Some(comma) = comma else {
            break;
        };
        let receiver = result[arguments_start..comma].trim();
        if receiver.is_empty()
            || !receiver
                .chars()
                .all(|character| character.is_alphanumeric() || matches!(character, '_' | '.'))
        {
            cursor = arguments_start;
            continue;
        }
        let mut rest = comma + 1;
        while result[rest..]
            .chars()
            .next()
            .is_some_and(char::is_whitespace)
        {
            rest += result[rest..].chars().next().unwrap().len_utf8();
        }
        let replacement = format!("{receiver}.{name}(");
        result.replace_range(start..rest, &replacement);
        cursor = start + replacement.len();
    }
    result
}

fn include_created_tests(
    test_tasks: &mut Vec<String>,
    plan: &Value,
    build_system: &str,
) -> Result<(), ClewError> {
    let mut created_tests = std::collections::BTreeSet::new();
    for operation in plan["operations"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|operation| operation["kind"] == "CREATE_FILE")
    {
        if let Some(route) = created_kotlin_test_route(operation)? {
            created_tests.insert(route);
        }
    }
    if created_tests.is_empty() {
        if build_system != "MAVEN" {
            validate_gradle_test_filter_ownership(test_tasks)?;
        }
        return Ok(());
    }
    if build_system == "MAVEN" {
        let stems = created_tests
            .into_iter()
            .map(|(_, stem)| stem)
            .collect::<std::collections::BTreeSet<_>>();
        if let Some(filter) = test_tasks
            .iter_mut()
            .find(|argument| argument.starts_with("-Dtest="))
        {
            let mut selected = filter
                .trim_start_matches("-Dtest=")
                .split(',')
                .map(str::to_owned)
                .collect::<std::collections::BTreeSet<_>>();
            selected.extend(stems);
            *filter = format!(
                "-Dtest={}",
                selected.into_iter().collect::<Vec<_>>().join(",")
            );
        } else {
            test_tasks.insert(
                0,
                format!("-Dtest={}", stems.into_iter().collect::<Vec<_>>().join(",")),
            );
        }
        return Ok(());
    }
    validate_gradle_test_filter_ownership(test_tasks)?;
    test_tasks.retain(|argument| argument != "test");
    for (test_task, stem) in created_tests {
        include_gradle_test_filter(test_tasks, &test_task, &format!("*{stem}"))?;
    }
    validate_gradle_test_filter_ownership(test_tasks)
}

fn created_kotlin_test_route(operation: &Value) -> Result<Option<(String, String)>, ClewError> {
    let path = operation
        .pointer("/target/fileId")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            ClewError::new(
                ErrorCode::InvalidInput,
                "CREATE_FILE test routing requires an exact target.fileId",
            )
        })?;
    if !has_kotlin_test_contour(path) {
        return Ok(None);
    }
    let route = task_context::gradle_test_route(path).ok_or_else(|| {
        ClewError::new(
            ErrorCode::InvalidInput,
            format!("CREATE_FILE Kotlin test path is not a canonical module-owned route: {path}"),
        )
    })?;
    let source = operation
        .pointer("/replacement/kotlin")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            ClewError::new(
                ErrorCode::InvalidInput,
                format!("CREATE_FILE Kotlin test has no exact replacement source: {path}"),
            )
        })?;
    if !has_top_level_kotlin_test_declaration(source, &route.1) {
        return Err(ClewError::new(
            ErrorCode::InvalidInput,
            format!(
                "CREATE_FILE Kotlin test {path} must declare a top-level class or object named {}",
                route.1
            ),
        ));
    }
    Ok(Some(route))
}

fn has_kotlin_test_contour(path: &str) -> bool {
    let normalized = path.replace('\\', "/");
    let components = normalized
        .split('/')
        .filter(|component| !component.is_empty() && *component != ".")
        .collect::<Vec<_>>();
    components
        .windows(3)
        .any(|window| window == ["src", "test", "kotlin"])
}

fn has_top_level_kotlin_test_declaration(source: &str, expected: &str) -> bool {
    let projection = kotlin_code_projection(source);
    let modifiers = [
        "public",
        "private",
        "protected",
        "internal",
        "expect",
        "actual",
        "final",
        "open",
        "abstract",
        "sealed",
        "external",
        "data",
        "enum",
        "annotation",
        "value",
    ];
    let mut brace_depth = 0usize;
    for line in projection.lines() {
        if brace_depth == 0 {
            let tokens = line
                .split(|character: char| !(character.is_alphanumeric() || character == '_'))
                .filter(|token| !token.is_empty())
                .collect::<Vec<_>>();
            if tokens.iter().enumerate().any(|(index, token)| {
                matches!(*token, "class" | "object")
                    && tokens.get(index + 1) == Some(&expected)
                    && tokens[..index]
                        .iter()
                        .all(|modifier| modifiers.contains(modifier))
            }) {
                return true;
            }
        }
        for character in line.chars() {
            match character {
                '{' => brace_depth = brace_depth.saturating_add(1),
                '}' => brace_depth = brace_depth.saturating_sub(1),
                _ => {}
            }
        }
    }
    false
}

fn kotlin_code_projection(source: &str) -> String {
    const NORMAL: u8 = 0;
    const LINE_COMMENT: u8 = 1;
    const BLOCK_COMMENT: u8 = 2;
    const STRING: u8 = 3;
    const RAW_STRING: u8 = 4;
    const CHARACTER: u8 = 5;

    let bytes = source.as_bytes();
    let mut output = Vec::with_capacity(bytes.len());
    let mut state = NORMAL;
    let mut block_depth = 0usize;
    let mut cursor = 0usize;
    while cursor < bytes.len() {
        let pair = bytes.get(cursor..cursor + 2);
        let triple = bytes.get(cursor..cursor + 3);
        match state {
            NORMAL if pair == Some(b"//") => {
                output.extend_from_slice(b"  ");
                cursor += 2;
                state = LINE_COMMENT;
            }
            NORMAL if pair == Some(b"/*") => {
                output.extend_from_slice(b"  ");
                cursor += 2;
                block_depth = 1;
                state = BLOCK_COMMENT;
            }
            NORMAL if triple == Some(b"\"\"\"") => {
                output.extend_from_slice(b"   ");
                cursor += 3;
                state = RAW_STRING;
            }
            NORMAL if bytes[cursor] == b'\"' => {
                output.push(b' ');
                cursor += 1;
                state = STRING;
            }
            NORMAL if bytes[cursor] == b'\'' => {
                output.push(b' ');
                cursor += 1;
                state = CHARACTER;
            }
            NORMAL => {
                output.push(bytes[cursor]);
                cursor += 1;
            }
            LINE_COMMENT if bytes[cursor] == b'\n' => {
                output.push(b'\n');
                cursor += 1;
                state = NORMAL;
            }
            LINE_COMMENT => {
                output.push(b' ');
                cursor += 1;
            }
            BLOCK_COMMENT if pair == Some(b"/*") => {
                output.extend_from_slice(b"  ");
                cursor += 2;
                block_depth += 1;
            }
            BLOCK_COMMENT if pair == Some(b"*/") => {
                output.extend_from_slice(b"  ");
                cursor += 2;
                block_depth -= 1;
                if block_depth == 0 {
                    state = NORMAL;
                }
            }
            BLOCK_COMMENT => {
                output.push(if bytes[cursor] == b'\n' { b'\n' } else { b' ' });
                cursor += 1;
            }
            RAW_STRING if triple == Some(b"\"\"\"") => {
                output.extend_from_slice(b"   ");
                cursor += 3;
                state = NORMAL;
            }
            RAW_STRING => {
                output.push(if bytes[cursor] == b'\n' { b'\n' } else { b' ' });
                cursor += 1;
            }
            STRING | CHARACTER if bytes[cursor] == b'\\' && cursor + 1 < bytes.len() => {
                output.extend_from_slice(b"  ");
                cursor += 2;
            }
            STRING if bytes[cursor] == b'\"' => {
                output.push(b' ');
                cursor += 1;
                state = NORMAL;
            }
            CHARACTER if bytes[cursor] == b'\'' => {
                output.push(b' ');
                cursor += 1;
                state = NORMAL;
            }
            STRING | CHARACTER => {
                output.push(if bytes[cursor] == b'\n' { b'\n' } else { b' ' });
                cursor += 1;
            }
            _ => unreachable!("known Kotlin lexical projection state"),
        }
    }
    String::from_utf8(output).expect("Kotlin lexical projection preserves UTF-8")
}

fn include_gradle_test_filter(
    arguments: &mut Vec<String>,
    owning_task: &str,
    selector: &str,
) -> Result<(), ClewError> {
    let Some(task_position) = arguments
        .iter()
        .position(|argument| argument == owning_task)
    else {
        arguments.extend([
            owning_task.to_owned(),
            "--tests".to_owned(),
            selector.to_owned(),
        ]);
        return Ok(());
    };
    let mut cursor = task_position + 1;
    let mut insertion = arguments.len();
    while cursor < arguments.len() {
        if arguments[cursor] == "--tests" {
            let selected = arguments.get(cursor + 1).ok_or_else(|| {
                ClewError::new(
                    ErrorCode::InvalidInput,
                    "Gradle --tests option has no selector",
                )
            })?;
            if selected == selector {
                return Ok(());
            }
            cursor += 2;
            continue;
        }
        if !arguments[cursor].starts_with('-') {
            insertion = cursor;
            break;
        }
        cursor += 1;
    }
    arguments.splice(
        insertion..insertion,
        ["--tests".to_owned(), selector.to_owned()],
    );
    Ok(())
}

fn validate_gradle_test_filter_ownership(arguments: &[String]) -> Result<(), ClewError> {
    if arguments.iter().any(|argument| argument == "--tests")
        && arguments.iter().any(|argument| argument == "test")
    {
        return Err(ClewError::new(
            ErrorCode::InvalidInput,
            "bare Gradle test selector cannot own or accompany targeted test filters",
        ));
    }
    let mut owning_task = None::<&str>;
    let mut cursor = 0usize;
    while cursor < arguments.len() {
        let argument = &arguments[cursor];
        if argument == "--tests" {
            if owning_task.is_none() {
                return Err(ClewError::new(
                    ErrorCode::InvalidInput,
                    "Gradle --tests filter has no exact owning test task",
                ));
            }
            if arguments.get(cursor + 1).is_none() {
                return Err(ClewError::new(
                    ErrorCode::InvalidInput,
                    "Gradle --tests option has no selector",
                ));
            }
            cursor += 2;
            continue;
        }
        if !argument.starts_with('-') {
            owning_task = (argument.starts_with(':') && argument.ends_with(":test"))
                .then_some(argument.as_str());
        }
        cursor += 1;
    }
    Ok(())
}

fn contains_identifier(source: &str, identifier: &str) -> bool {
    source.match_indices(identifier).any(|(start, _)| {
        let is_identifier = |character: char| character.is_alphanumeric() || character == '_';
        source[..start]
            .chars()
            .next_back()
            .is_none_or(|character| !is_identifier(character))
            && source[start + identifier.len()..]
                .chars()
                .next()
                .is_none_or(|character| !is_identifier(character))
    })
}
fn parse_error(e: impl std::fmt::Display) -> ClewError {
    ClewError::new(ErrorCode::InvalidInput, e.to_string())
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
mod task_plan_tests {
    use super::*;

    fn conditional_decision() -> Value {
        json!({
            "schema":TYPED_GOAL_BINDING_DECISION_SCHEMA,
            "status":"CONDITIONAL",
            "revision":"abc",
            "goalFingerprint":format!("sha256:{}", "a".repeat(64)),
            "establishedEvidenceFingerprint":format!("sha256:{}", "b".repeat(64)),
            "unresolvedObligations":[{
                "id":"verify-call-target-identity",
                "code":"VERIFY_CALL_TARGET_IDENTITY",
                "subject":["p/target"],
                "establishedAuthority":"SOURCE_STRUCTURAL",
                "requiredAuthority":"COMPILER_EXACT",
                "acceptableVerifiers":["COMPILER_ARGUMENT_MAPPING"],
                "publicationBlocking":true
            }]
        })
    }

    #[test]
    fn conditional_decision_is_revision_bound_and_always_blocks_publication() {
        let decision = conditional_decision();
        let obligations = validate_conditional_decision(&decision, "abc").unwrap();
        assert_eq!(obligations.len(), 1);
        assert_eq!(obligations[0]["publicationBlocking"], true);

        let mut stale = decision.clone();
        stale["revision"] = json!("other");
        assert!(validate_conditional_decision(&stale, "abc").is_err());

        let mut forged = decision;
        forged["unresolvedObligations"][0]["publicationBlocking"] = json!(false);
        assert!(validate_conditional_decision(&forged, "abc").is_err());
    }

    #[test]
    fn legacy_task_apply_is_not_public() {
        assert!(
            Cli::try_parse_from([
                "clew",
                "task-apply",
                "--repo",
                "/repo",
                "--context",
                "/context.json",
                "--edit-plan",
                "/plan.json",
                "--target-ref",
                "main",
                "--transaction-id",
                "tx:2f1f8596-04ed-5c0d-a1fc-804f56a0a728",
            ])
            .is_err()
        );
    }

    #[test]
    fn only_managed_workflow_command_families_are_public() {
        for legacy in [
            "doctor",
            "schema",
            "project",
            "index",
            "resolve",
            "cfg",
            "slice",
            "projection",
            "prove",
            "apply",
            "edit",
            "tx",
        ] {
            assert!(
                Cli::try_parse_from(["clew", legacy]).is_err(),
                "legacy command {legacy} remained public"
            );
        }
        for managed in ["session", "context", "plan", "task-run"] {
            let error = match Cli::try_parse_from(["clew", managed]) {
                Ok(_) => panic!("managed command {managed} unexpectedly accepted no subcommand"),
                Err(error) => error,
            };
            assert_eq!(
                error.kind(),
                clap::error::ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand
            );
        }
    }

    #[test]
    fn compiler_index_root_is_not_a_public_cli_authority() {
        for arguments in [
            vec![
                "clew",
                "--compiler-index-root",
                "/private/tmp/codeclew-index",
                "index",
                "--repo",
                "/repo",
            ],
            vec![
                "clew",
                "index",
                "--repo",
                "/repo",
                "--compiler-index-root",
                "/private/tmp/codeclew-index",
            ],
        ] {
            assert!(Cli::try_parse_from(arguments).is_err());
        }
    }

    #[test]
    fn recover_is_an_explicit_session_operation_bound_to_one_run() {
        assert!(
            Cli::try_parse_from([
                "clew",
                "session",
                "recover",
                "--session",
                "session:authority",
                "--run",
                "run:request",
            ])
            .is_ok()
        );
        assert!(
            Cli::try_parse_from(["clew", "session", "recover", "--run", "run:request"]).is_err()
        );
        assert!(
            Cli::try_parse_from(["clew", "task-run", "cancel", "--run", "run:request"]).is_ok()
        );
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
        assert!(process_start_token(child.id()).unwrap().is_none());
    }

    #[test]
    fn normalizes_compact_rewrites_and_imports_created_cross_package_types() {
        let mut plan = json!({"edits":[
            {
                "op":"CREATE_FILE",
                "target":{"fileId":"src/main/kotlin/com/acme/contracts/Entity.kt"},
                "replacement":{"kotlin":"package com.acme.contracts\n\ninterface Entity {\n    val id: String\n}"}
            },
            {
                "kind":"REWRITE_DECLARATION",
                "target":{"fileId":"src/main/kotlin/com/acme/service/Service.kt"},
                "preconditions":{"substitutions":[{"old":"Old) {", "new":"com.acme.contracts.Entity(); Old) : Entity {"}]}
            }
        ]});

        normalize_task_plan(&mut plan).unwrap();
        inject_created_type_imports(&mut plan).unwrap();
        inject_created_contract_overrides(&mut plan).unwrap();

        let operations = plan["operations"].as_array().unwrap();
        assert_eq!(operations.len(), 3);
        assert_eq!(operations[1]["kind"], "ADD_IMPORT");
        assert_eq!(
            operations[1]["replacement"]["kotlin"],
            "com.acme.contracts.Entity"
        );
        assert_eq!(operations[2]["replacement"]["kotlin"], "");
        assert_eq!(operations[2]["opId"], "task-op-2");
        assert_eq!(
            operations[2]["preconditions"]["substitutions"][1]["new"],
            "override val id:"
        );
    }

    #[test]
    fn imports_top_level_explicit_targets_used_by_a_workflow_rewrite() {
        let identity = |name: &str| {
            json!({
                "package":"com.acme.options",
                "containingDeclarations":[],
                "declarationName":name,
                "receiverTypes":if name == "applyOptions" {vec!["String"]} else {Vec::<&str>::new()}
            })
            .to_string()
        };
        let context = json!({"editSurfaces":[
            {
                "name":"readOptions",
                "role":"EXPLICIT_TARGET",
                "declarationTarget":{"ownerSymbolId":identity("readOptions")}
            },
            {
                "name":"applyOptions",
                "role":"EXPLICIT_TARGET",
                "declarationTarget":{"ownerSymbolId":identity("applyOptions")}
            }
        ]});
        let mut plan = json!({"operations":[{
            "opId":"rewrite-workflow",
            "kind":"REWRITE_DECLARATION",
            "target":{"fileId":"src/main/kotlin/com/acme/app/Runner.kt"},
            "replacement":{"kotlin":""},
            "preconditions":{"substitutions":[{
                "old":"val records = load()",
                "new":"val options = readOptions()\nval records = load().map { com.acme.options.applyOptions(it, options) }",
                "occurrences":1
            }]},
            "postconditions":{}
        }]});

        inject_explicit_target_imports(&mut plan, &context).unwrap();

        let operations = plan["operations"].as_array().unwrap();
        assert_eq!(operations.len(), 3);
        assert_eq!(operations[0]["kind"], "ADD_IMPORT");
        assert_eq!(
            operations[0]["replacement"]["kotlin"],
            "com.acme.options.readOptions"
        );
        assert_eq!(operations[1]["kind"], "ADD_IMPORT");
        assert_eq!(
            operations[1]["replacement"]["kotlin"],
            "com.acme.options.applyOptions"
        );
        assert_eq!(operations[2]["opId"], "rewrite-workflow");
        assert_eq!(
            operations[2]["preconditions"]["substitutions"][0]["new"],
            "val options = readOptions()\nval records = load().map { it.applyOptions(options) }"
        );
    }

    #[test]
    fn created_tests_are_added_to_gradle_and_maven_validation() {
        let plan = json!({"operations":[{
            "kind":"CREATE_FILE",
            "target":{"fileId":"src/test/kotlin/com/acme/RunnerTest.kt"},
            "replacement":{"kotlin":"package com.acme\n\nclass RunnerTest"}
        }]});
        let mut gradle = vec![
            "cleanTest".into(),
            ":test".into(),
            "--tests".into(),
            "*ExistingTest".into(),
        ];
        include_created_tests(&mut gradle, &plan, "GRADLE").unwrap();
        assert_eq!(
            gradle,
            vec![
                "cleanTest",
                ":test",
                "--tests",
                "*ExistingTest",
                "--tests",
                "*RunnerTest"
            ]
        );

        let mut maven = vec!["-Dtest=ExistingTest".into(), "test".into()];
        include_created_tests(&mut maven, &plan, "MAVEN").unwrap();
        assert_eq!(maven, vec!["-Dtest=ExistingTest,RunnerTest", "test"]);

        let module_plan = json!({"operations":[{
            "kind":"CREATE_FILE",
            "target":{"fileId":"workers/kotlin21/src/test/kotlin/dev/acme/BtaBackendTest.kt"},
            "replacement":{"kotlin":"package dev.acme\n\nclass BtaBackendTest"}
        }]});
        let mut module_tasks = Vec::new();
        include_created_tests(&mut module_tasks, &module_plan, "GRADLE").unwrap();
        assert_eq!(
            module_tasks,
            vec![":workers:kotlin21:test", "--tests", "*BtaBackendTest"]
        );

        let mut default_gradle_tasks = vec!["cleanTest".into(), "test".into()];
        include_created_tests(&mut default_gradle_tasks, &module_plan, "GRADLE").unwrap();
        assert_eq!(
            default_gradle_tasks,
            vec![
                "cleanTest",
                ":workers:kotlin21:test",
                "--tests",
                "*BtaBackendTest"
            ]
        );
    }

    #[test]
    fn created_k21_test_shares_only_its_owning_module_task_with_surfaced_test() {
        let plan = json!({"operations":[{
            "kind":"CREATE_FILE",
            "target":{"fileId":"workers/kotlin21/src/test/kotlin/dev/semanticthread/worker/BtaIncrementalBackend21Test.kt"},
            "replacement":{"kotlin":"package dev.semanticthread.worker\n\nclass BtaIncrementalBackend21Test"}
        }]});
        let mut tasks = vec![
            "cleanTest".into(),
            ":workers:kotlin21:test".into(),
            "--tests".into(),
            "*K2FactGenerationStore21Test".into(),
        ];

        include_created_tests(&mut tasks, &plan, "GRADLE").unwrap();

        assert_eq!(
            tasks,
            vec![
                "cleanTest",
                ":workers:kotlin21:test",
                "--tests",
                "*K2FactGenerationStore21Test",
                "--tests",
                "*BtaIncrementalBackend21Test",
            ]
        );
        assert!(!tasks.iter().any(|argument| argument == "test"));
    }

    #[test]
    fn created_gradle_test_is_inserted_before_the_next_module_route() {
        let plan = json!({"operations":[{
            "kind":"CREATE_FILE",
            "target":{"fileId":"workers/kotlin/src/test/kotlin/dev/semanticthread/worker/CommonCreatedTest.kt"},
            "replacement":{"kotlin":"package dev.semanticthread.worker\n\ninternal class CommonCreatedTest"}
        }]});
        let mut tasks = vec![
            "cleanTest".into(),
            ":workers:kotlin:test".into(),
            "--tests".into(),
            "*ExistingCommonTest".into(),
            ":workers:kotlin21:test".into(),
            "--tests".into(),
            "*ExistingK21Test".into(),
        ];

        include_created_tests(&mut tasks, &plan, "GRADLE").unwrap();

        assert_eq!(
            tasks,
            vec![
                "cleanTest",
                ":workers:kotlin:test",
                "--tests",
                "*ExistingCommonTest",
                "--tests",
                "*CommonCreatedTest",
                ":workers:kotlin21:test",
                "--tests",
                "*ExistingK21Test",
            ]
        );
    }

    #[test]
    fn created_common_runtime_test_is_added_alongside_existing_context_filter() {
        let plan = json!({"operations":[{
            "kind":"CREATE_FILE",
            "target":{"fileId":"workers/kotlin/src/test/kotlin/dev/semanticthread/worker/IncrementalK2RuntimeTest.kt"},
            "replacement":{"kotlin":"package dev.semanticthread.worker\n\nclass IncrementalK2RuntimeTest"}
        }]});
        let mut tasks = vec![
            "cleanTest".into(),
            ":workers:kotlin:test".into(),
            "--tests".into(),
            "*TransportProjectModelCommandTest".into(),
        ];

        include_created_tests(&mut tasks, &plan, "GRADLE").unwrap();

        assert_eq!(
            tasks,
            vec![
                "cleanTest",
                ":workers:kotlin:test",
                "--tests",
                "*TransportProjectModelCommandTest",
                "--tests",
                "*IncrementalK2RuntimeTest",
            ]
        );
    }

    #[test]
    fn created_common_runtime_test_replaces_bare_model_fallback_gate() {
        let plan = json!({"operations":[{
            "kind":"CREATE_FILE",
            "target":{"fileId":"workers/kotlin/src/test/kotlin/dev/semanticthread/worker/IncrementalK2RuntimeTest.kt"},
            "replacement":{"kotlin":"package dev.semanticthread.worker\n\nobject IncrementalK2RuntimeTest"}
        }]});
        let mut tasks = vec!["cleanTest".into(), "test".into()];

        include_created_tests(&mut tasks, &plan, "GRADLE").unwrap();

        assert_eq!(
            tasks,
            vec![
                "cleanTest",
                ":workers:kotlin:test",
                "--tests",
                "*IncrementalK2RuntimeTest",
            ]
        );
    }

    #[test]
    fn created_test_routes_are_sorted_deduplicated_and_do_not_repeat_existing_filter() {
        let operation = |name: &str| {
            json!({
                "kind":"CREATE_FILE",
                "target":{"fileId":format!("workers/kotlin/src/test/kotlin/dev/acme/{name}.kt")},
                "replacement":{"kotlin":format!("package dev.acme\n\nclass {name}")}
            })
        };
        let plan = json!({"operations":[
            operation("ZuluTest"),
            operation("AlphaTest"),
            operation("ZuluTest"),
        ]});
        let mut tasks = vec![
            "cleanTest".into(),
            ":workers:kotlin:test".into(),
            "--tests".into(),
            "*AlphaTest".into(),
        ];

        include_created_tests(&mut tasks, &plan, "GRADLE").unwrap();

        assert_eq!(
            tasks,
            vec![
                "cleanTest",
                ":workers:kotlin:test",
                "--tests",
                "*AlphaTest",
                "--tests",
                "*ZuluTest",
            ]
        );
    }

    #[test]
    fn created_kotlin_test_paths_fail_closed_when_not_canonical_or_routable() {
        for path in [
            "workers/kotlin/src/test/kotlin/../RuntimeTest.kt",
            "workers\\kotlin\\src\\test\\kotlin\\dev\\acme\\RuntimeTest.kt",
            "/workers/kotlin/src/test/kotlin/dev/acme/RuntimeTest.kt",
            "workers/kotlin/src/test/kotlin/dev/acme/RuntimeTest.java",
            "workers/kotlin/src/test/kotlin/dev/acme/RuntimeSpec.kt",
            "src/test/kotlin",
            "workers/src/test/kotlin/dev/src/test/kotlin/AmbiguousTest.kt",
        ] {
            let plan = json!({"operations":[{
                "kind":"CREATE_FILE",
                "target":{"fileId":path},
                "replacement":{"kotlin":"class RuntimeTest"}
            }]});
            let mut tasks = vec!["cleanTest".into()];

            let error = include_created_tests(&mut tasks, &plan, "GRADLE").unwrap_err();

            assert_eq!(error.code, ErrorCode::InvalidInput, "path={path}");
            assert!(error.message.contains("canonical module-owned route"));
            assert_eq!(tasks, vec!["cleanTest"]);
        }
    }

    #[test]
    fn created_kotlin_test_requires_matching_top_level_type_and_ignores_main_source() {
        let unmatched = json!({"operations":[{
            "kind":"CREATE_FILE",
            "target":{"fileId":"workers/kotlin/src/test/kotlin/dev/acme/RuntimeTest.kt"},
            "replacement":{"kotlin":"// class RuntimeTest\nval decoy = \"class RuntimeTest\"\nclass OtherTest"}
        }]});
        let mut tasks = vec!["cleanTest".into()];
        let error = include_created_tests(&mut tasks, &unmatched, "GRADLE").unwrap_err();
        assert_eq!(error.code, ErrorCode::InvalidInput);
        assert!(
            error
                .message
                .contains("top-level class or object named RuntimeTest")
        );

        let main_source = json!({"operations":[{
            "kind":"CREATE_FILE",
            "target":{"fileId":"workers/kotlin/src/main/kotlin/dev/acme/Runtime.kt"},
            "replacement":{"kotlin":"package dev.acme\n\nclass Runtime"}
        }]});
        include_created_tests(&mut tasks, &main_source, "GRADLE").unwrap();
        assert_eq!(tasks, vec!["cleanTest"]);
    }

    #[test]
    fn bare_gradle_test_selector_cannot_own_a_targeted_filter() {
        let mut tasks = vec![
            "cleanTest".into(),
            "test".into(),
            "--tests".into(),
            "*UnownedTest".into(),
        ];
        let error =
            include_created_tests(&mut tasks, &json!({"operations":[]}), "GRADLE").unwrap_err();

        assert_eq!(error.code, ErrorCode::InvalidInput);
        assert!(error.message.contains("bare Gradle test selector"));
    }

    #[test]
    fn normalizes_and_expands_only_emitted_model_input_targets() {
        let exact_hash = canonical::hash_bytes(b"plugins {}\n");
        let context = json!({
            "editSurfaces":[],
            "contracts":[],
            "tests":[],
            "modelInputSurfaces":[{
                "targetId":"M1",
                "path":"workers/kotlin21/build.gradle.kts",
                "sourceText":"plugins {}\n",
                "modelInputTarget":{
                    "anchorId":"model-input:one",
                    "fileId":"workers/kotlin21/build.gradle.kts",
                    "exactTextHash":exact_hash,
                    "syntaxKind":"MODEL_INPUT_FILE",
                    "semanticInputManifestHash":"sha256:manifest"
                }
            }]
        });
        let mut plan = json!({"operations":[{
            "kind":"REPLACE_MODEL_INPUT",
            "target":{"targetId":"M1"},
            "newLines":["plugins {", "    kotlin(\"jvm\")", "}", ""]
        }]});

        normalize_task_plan(&mut plan).unwrap();
        expand_task_targets(&mut plan, &context).unwrap();

        assert_eq!(
            plan["operations"][0]["replacement"]["kotlin"],
            "plugins {\n    kotlin(\"jvm\")\n}\n"
        );
        assert_eq!(
            plan["operations"][0]["target"]["fileId"],
            "workers/kotlin21/build.gradle.kts"
        );
        assert_eq!(plan["operations"][0]["target"]["exactTextHash"], exact_hash);

        for rejected in [
            json!({"operations":[{
                "kind":"REPLACE_MODEL_INPUT",
                "target":{"targetId":"workers/kotlin21/build.gradle.kts"},
                "newLines":["plugins {}"]
            }]}),
            json!({"operations":[{
                "kind":"REPLACE_MODEL_INPUT",
                "target":{"targetId":"M1"},
                "path":"workers/kotlin21/build.gradle.kts",
                "newLines":["plugins {}"]
            }]}),
            json!({"operations":[{
                "kind":"REPLACE_MODEL_INPUT",
                "target":{"targetId":"M1"},
                "newLines":["plugins {}\nrepositories {}"]
            }]}),
        ] {
            let mut rejected = rejected;
            let result = normalize_task_plan(&mut rejected)
                .and_then(|_| expand_task_targets(&mut rejected, &context));
            assert!(result.is_err());
        }
    }

    #[test]
    fn normalizes_multiline_plan_fields_without_escaped_newlines() {
        let mut plan = json!({"operations":[
            {
                "kind":"CREATE_FILE",
                "target":{"targetId":"file:src/main/kotlin/com/acme/Entity.kt"},
                "path":"src/main/kotlin/com/acme/Entity.kt",
                "newLines":"    package com.acme\n\n    interface Entity"
            },
            {
                "kind":"REWRITE_DECLARATION",
                "substitutions":[{
                    "oldLines":["        fun old() {", "        }"],
                    "newLines":["        fun new() {", "            call()", "        }"]
                }]
            },
            {
                "kind":"REWRITE_DECLARATION",
                "target":null,
                "old":"call()",
                "new":"otherCall()"
            }
        ]});

        normalize_task_plan(&mut plan).unwrap();

        assert_eq!(plan["operations"].as_array().unwrap().len(), 2);
        assert_eq!(
            plan["operations"][0]["replacement"]["kotlin"],
            "package com.acme\n\ninterface Entity"
        );
        assert_eq!(plan["operations"][0]["kind"], "CREATE_FILE");
        assert_eq!(
            plan["operations"][0]["target"]["fileId"],
            "src/main/kotlin/com/acme/Entity.kt"
        );
        assert_eq!(
            plan["operations"][1]["preconditions"]["substitutions"][0]["old"],
            "fun old() {\n}"
        );
        assert_eq!(
            plan["operations"][1]["preconditions"]["substitutions"][0]["new"],
            "fun new() {\n    call()\n}"
        );
        assert_eq!(
            plan["operations"][1]["preconditions"]["substitutions"][0]["lineMode"],
            true
        );
        assert_eq!(
            plan["operations"][1]["preconditions"]["substitutions"][1]["new"],
            "otherCall()"
        );
    }

    #[test]
    fn does_not_treat_a_parameter_type_as_a_created_contract_implementation() {
        let mut plan = json!({"operations":[
            {
                "kind":"CREATE_FILE",
                "target":{"fileId":"src/main/kotlin/com/acme/Entity.kt"},
                "replacement":{"kotlin":"package com.acme\n\ninterface Entity {\n    val id: String\n}"}
            },
            {
                "kind":"REWRITE_DECLARATION",
                "target":{},
                "preconditions":{"substitutions":[{
                    "old":"product: Old?", "new":"product: Entity?"
                }]},
                "replacement":{"kotlin":""}
            }
        ]});

        inject_created_contract_overrides(&mut plan).unwrap();

        assert_eq!(
            plan["operations"][1]["preconditions"]["substitutions"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
    }
}
