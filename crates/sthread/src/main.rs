use clap::{Args, Parser, Subcommand, ValueEnum};
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
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
    Index(RepoArgs),
    Resolve {
        #[command(subcommand)]
        command: ResolveCommand,
    },
    Cfg(SymbolArgs),
    Slice(SliceArgs),
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
    #[arg(long)]
    symbol: String,
    #[arg(long, value_enum, default_value = "both")]
    direction: DirectionArg,
    #[arg(long, default_value_t = 200)]
    max_nodes: usize,
    #[arg(long)]
    output: Option<PathBuf>,
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
                &json!({"repo":absolute(&args.repo)?}),
            )
        }),
        Command::Index(args) => with_worker(&workspace, |w| {
            let repo = absolute(&args.repo)?;
            let facts = w.request(RequestKind::IndexFiles, &json!({"repo":repo}))?;
            let mut index = RepositoryIndex::open(&repo)?;
            let persistent_hash = index.update(&facts)?;
            Ok(
                json!({"schema":"semantic-index-result/0.1","workerIndexHash":facts["indexHash"],"persistentIndexHash":persistent_hash,"files":facts["files"].as_array().map_or(0,Vec::len)}),
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
                    Err(e)
                }
            }
        }),
        Command::Tx {
            command: TxCommand::Commit(args),
        } => with_worker(&workspace, |w| {
            let repo = absolute(&args.repo)?;
            let mut tx: Transaction = read_json(&args.file)?;
            transaction::commit(&repo, &mut tx, &args.target_ref, w)
        }),
        Command::Tx {
            command: TxCommand::Inspect(args),
        } => transaction::ledger(&absolute(&args.repo)?)?.inspect(&args.transaction_id),
    }
}

fn slice_command(worker: &mut WorkerClient, args: SliceArgs) -> Result<Value, SthreadError> {
    let repo = absolute(&args.repo)?;
    let project = worker.request(RequestKind::OpenProject, &json!({"repo":repo}))?;
    let raw = worker.request(
        RequestKind::BuildLocalGraph,
        &json!({"repo":repo,"symbol":args.symbol}),
    )?;
    let graph: LocalGraph = serde_json::from_value(raw).map_err(parse_error)?;
    let graph = graph::enrich(graph);
    let seed_id = graph
        .nodes
        .iter()
        .filter(|n| n.kind == "RETURN")
        .map(|n| n.id.clone())
        .next_back()
        .unwrap_or_else(|| "exit".into());
    let seed_node = graph.nodes.iter().find(|n| n.id == seed_id);
    let seed = json!({"kind":"FUNCTION_RETURN","symbol":args.symbol,"nodeId":seed_id,"anchor":seed_node.and_then(|n|n.origin.clone())});
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
        compiler_version: "2.4.10".into(),
    };
    let thread = graph::slice(&graph, &seed_id, policy, snapshot, seed)
        .map_err(|e| SthreadError::new(ErrorCode::Internal, e.to_string()))?;
    write_optional(args.output.as_deref(), &thread)?;
    serde_json::to_value(thread).map_err(parse_error)
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
