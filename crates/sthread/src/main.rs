use clap::{Args, Parser, Subcommand, ValueEnum};
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use sthread::agent_context;
use sthread::canonical;
use sthread::error::{ErrorCode, SthreadError};
use sthread::graph;
use sthread::index::RepositoryIndex;
use sthread::model::*;
use sthread::proto::RequestKind;
use sthread::transaction;
use sthread::worker::{WorkerClient, workspace_root};

#[derive(Parser)]
#[command(
    name = "sthread",
    version,
    about = "Semantic Thread Platform Kotlin MVP"
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
    Doctor,
    Project {
        #[command(subcommand)]
        command: ProjectCommand,
    },
    Index(IndexArgs),
    Resolve {
        #[command(subcommand)]
        command: ResolveCommand,
    },
    Cfg(SymbolArgs),
    Slice(SliceArgs),
    #[command(
        name = "agent-context",
        visible_alias = "context",
        about = "Build one bounded edit-ready semantic context pack for an agent"
    )]
    AgentContext(AgentContextArgs),
    Edit {
        #[command(subcommand)]
        command: EditCommand,
    },
    Tx {
        #[command(subcommand)]
        command: TxCommand,
    },
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
struct AgentContextArgs {
    #[arg(long)]
    repo: PathBuf,
    #[arg(long, default_value = ":/main")]
    compilation: String,
    #[arg(long = "term", visible_alias = "symbol", required = true)]
    terms: Vec<String>,
    #[arg(long, default_value_t = 12_288)]
    max_bytes: usize,
    #[arg(long, default_value = ".semantic-thread/agent-context.json")]
    evidence: PathBuf,
    #[arg(long, hide = true)]
    output: Option<PathBuf>,
    #[arg(long = "max-nodes", hide = true)]
    _max_nodes: Option<usize>,
}
#[derive(Clone, ValueEnum)]
enum DirectionArg {
    Forward,
    Backward,
    Both,
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
    let started = std::time::Instant::now();
    let result = run(cli);
    eprintln!(
        "{}",
        serde_json::to_string(&json!({
            "event":"request_completed",
            "durationMs":started.elapsed().as_millis(),
            "success":result.is_ok()
        }))
        .unwrap_or_default()
    );
    match result {
        Ok(value) => {
            println!("{}", canonical::pretty(&value).unwrap());
            ExitCode::SUCCESS
        }
        Err(error) => {
            println!(
                "{}",
                canonical::pretty(&json!({"schema":"semantic-error/0.1","error":error})).unwrap()
            );
            ExitCode::from(exit_code(&error.code))
        }
    }
}

fn run(cli: Cli) -> Result<Value, SthreadError> {
    let workspace = workspace_root();
    match cli.command {
        Command::Doctor => {
            let worker = WorkerClient::start(&workspace)?;
            let result = json!({"schema":"semantic-doctor/0.1","status":"OK","rustCore":env!("CARGO_PKG_VERSION"),"worker":{"language":worker.capabilities.language,"version":worker.capabilities.worker_version,"compilerVersion":worker.capabilities.compiler_version,"operations":worker.capabilities.supported_operations}});
            worker.shutdown()?;
            Ok(result)
        }
        Command::Project {
            command: ProjectCommand::Inspect(args),
        } => with_worker(&workspace, |w| {
            w.request(
                RequestKind::OpenProject,
                &json!({"repo":absolute(&args.repo)?,"compilation":args.compilation}),
            )
        }),
        Command::Index(args) => with_worker(&workspace, |w| {
            let repo = absolute(&args.repo)?;
            let project = w.request(
                RequestKind::OpenProject,
                &json!({"repo":repo,"compilation":args.compilation}),
            )?;
            let facts = w.request(
                RequestKind::IndexFiles,
                &json!({"repo":repo,"compilation":args.compilation,"syntaxOnly":args.syntax_only,"files":args.files}),
            )?;
            let syntax_storage = args
                .syntax_only
                .then(|| format!("{}#syntax", args.compilation.as_deref().unwrap_or(":/main")));
            let mut index = RepositoryIndex::open_compilation(
                &repo,
                syntax_storage.as_deref().or(args.compilation.as_deref()),
            )?;
            let persistent_hash = index.update(&facts)?;
            let invalidations = index.invalidations()?;
            Ok(
                json!({"schema":"semantic-index-result/0.1","projectModelHash":project["projectModelHash"],"workerIndexHash":facts["indexHash"],"persistentIndexHash":persistent_hash,"files":facts["files"].as_array().map_or(0,Vec::len),"invalidations":invalidations}),
            )
        }),
        Command::Resolve {
            command: ResolveCommand::Symbol(args),
        } => with_worker(&workspace, |w| {
            w.request(
                RequestKind::ResolveSymbol,
                &json!({"repo":absolute(&args.repo)?,"symbol":args.symbol}),
            )
        }),
        Command::Resolve {
            command: ResolveCommand::Expression(args),
        } => with_worker(&workspace, |w| {
            w.request(
                RequestKind::ResolveExpression,
                &json!({"repo":absolute(&args.repo)?,"file":args.file,"offset":args.offset}),
            )
        }),
        Command::Cfg(args) => with_worker(&workspace, |w| {
            let raw = w.request(
                RequestKind::BuildLocalGraph,
                &json!({"repo":absolute(&args.repo)?,"symbol":args.symbol}),
            )?;
            let graph: LocalGraph = serde_json::from_value(raw).map_err(parse_error)?;
            serde_json::to_value(graph::enrich(graph)).map_err(parse_error)
        }),
        Command::Slice(args) => with_worker(&workspace, |w| slice_command(w, args)),
        Command::AgentContext(args) => with_worker(&workspace, |worker| {
            let repo = absolute(&args.repo)?;
            let project = worker.request(
                RequestKind::OpenProject,
                &json!({"repo":repo,"compilation":args.compilation}),
            )?;
            let index_facts = worker.request(
                RequestKind::IndexFiles,
                &json!({"repo":repo,"compilation":args.compilation,"syntaxOnly":true}),
            )?;
            let storage_compilation = format!("{}#agent-context-syntax", args.compilation);
            let mut repository_index =
                RepositoryIndex::open_compilation(&repo, Some(&storage_compilation))?;
            repository_index.update(&index_facts)?;
            let selection = agent_context::select(&repo, &index_facts, &args.terms)?;
            let resolutions = selection
                .function_symbols()
                .into_iter()
                .take(6)
                .map(|symbol| {
                    worker.request(
                        RequestKind::ResolveSymbol,
                        &json!({"repo":repo,"compilation":args.compilation,"symbol":symbol}),
                    )
                })
                .collect::<Result<Vec<_>, _>>()?;
            let evidence_path = if args.evidence.is_absolute() {
                args.evidence
            } else {
                repo.join(args.evidence)
            };
            let base_revision = git_head(&repo)?;
            let (context, evidence) = agent_context::build(agent_context::AgentContextBuild {
                repo: &repo,
                terms: &args.terms,
                compilation: &args.compilation,
                project: &project,
                index_facts: &index_facts,
                selection: &selection,
                resolutions: &resolutions,
                base_revision: &base_revision,
                evidence_path: &evidence_path,
                max_bytes: args.max_bytes,
            })?;
            write_artifact(&evidence_path, &evidence)?;
            if let Some(output) = args.output {
                let output_path = if output.is_absolute() {
                    output
                } else {
                    repo.join(output)
                };
                write_artifact(&output_path, &context)?;
            }
            Ok(context)
        }),
        Command::Edit {
            command: EditCommand::Preview(args),
        } => with_worker(&workspace, |w| {
            let repo = absolute(&args.repo)?;
            let thread: ThreadIr = read_json(&args.thread)?;
            let edit: EditIr = read_json(&args.edit)?;
            let report = transaction::preview(&repo, &thread, &edit, w)?;
            write_optional(args.output.as_deref(), &report)?;
            serde_json::to_value(report).map_err(parse_error)
        }),
        Command::Tx {
            command: TxCommand::Validate(args),
        } => with_worker(&workspace, |w| {
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
        Command::Tx {
            command: TxCommand::Commit(args),
        } => with_worker(&workspace, |w| {
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
        Command::Tx {
            command: TxCommand::Inspect(args),
        } => transaction::ledger(&absolute(&args.repo)?)?.inspect(&args.transaction_id),
    }
}

fn slice_command(worker: &mut WorkerClient, args: SliceArgs) -> Result<Value, SthreadError> {
    let output = args.output.clone();
    let thread = build_thread(worker, args)?;
    write_optional(output.as_deref(), &thread)?;
    serde_json::to_value(thread).map_err(parse_error)
}

fn build_thread(worker: &mut WorkerClient, args: SliceArgs) -> Result<ThreadIr, SthreadError> {
    let repo = absolute(&args.repo)?;
    let project = worker.request(
        RequestKind::OpenProject,
        &json!({"repo":repo,"compilation":args.compilation}),
    )?;
    let index_facts = worker.request(
        RequestKind::IndexFiles,
        &json!({"repo":repo,"compilation":args.compilation}),
    )?;
    let mut repository_index = RepositoryIndex::open_compilation(&repo, Some(&args.compilation))?;
    let index_snapshot = repository_index.update(&index_facts)?;
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
            SthreadError::new(
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
                SthreadError::new(
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
                SthreadError::new(
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
        .map_err(|e| SthreadError::new(ErrorCode::Internal, e.to_string()))?;
    Ok(thread)
}

fn with_worker<F>(workspace: &Path, action: F) -> Result<Value, SthreadError>
where
    F: FnOnce(&mut WorkerClient) -> Result<Value, SthreadError>,
{
    let mut worker = WorkerClient::start(workspace)?;
    let result = action(&mut worker);
    let shutdown = worker.shutdown();
    match (result, shutdown) {
        (Ok(v), Ok(())) => Ok(v),
        (Err(e), _) => Err(e),
        (_, Err(e)) => Err(e),
    }
}
fn absolute(path: &Path) -> Result<PathBuf, SthreadError> {
    path.canonicalize()
        .map_err(|e| SthreadError::new(ErrorCode::InvalidInput, format!("{}: {e}", path.display())))
}
fn git_head(repo: &Path) -> Result<String, SthreadError> {
    let output = std::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(repo)
        .output()
        .map_err(|e| SthreadError::new(ErrorCode::InvalidInput, e.to_string()))?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).trim().into())
    } else {
        Err(SthreadError::new(
            ErrorCode::InvalidInput,
            "repository must have a committed Git HEAD",
        ))
    }
}
fn read_json<T: DeserializeOwned>(path: &Path) -> Result<T, SthreadError> {
    let bytes = std::fs::read(path)
        .map_err(|e| SthreadError::new(ErrorCode::InvalidInput, e.to_string()))?;
    serde_json::from_slice(&bytes).map_err(parse_error)
}
fn write_optional<T: serde::Serialize>(path: Option<&Path>, value: &T) -> Result<(), SthreadError> {
    if let Some(path) = path {
        std::fs::write(
            path,
            canonical::pretty(value)
                .map_err(|e| SthreadError::new(ErrorCode::Internal, e.to_string()))?,
        )
        .map_err(|e| SthreadError::new(ErrorCode::Internal, e.to_string()))?
    }
    Ok(())
}
fn write_artifact<T: serde::Serialize>(path: &Path, value: &T) -> Result<(), SthreadError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| SthreadError::new(ErrorCode::Internal, e.to_string()))?;
    }
    std::fs::write(
        path,
        canonical::pretty(value)
            .map_err(|e| SthreadError::new(ErrorCode::Internal, e.to_string()))?,
    )
    .map_err(|e| SthreadError::new(ErrorCode::Internal, e.to_string()))
}
fn parse_error(e: impl std::fmt::Display) -> SthreadError {
    SthreadError::new(ErrorCode::InvalidInput, e.to_string())
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
