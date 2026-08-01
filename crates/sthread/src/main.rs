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
use sthread::task_context;
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
    #[command(
        name = "task-apply",
        about = "Apply one graph-derived multi-file task edit and commit it after clean validation"
    )]
    TaskApply(TaskApplyArgs),
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
    #[arg(long, default_value = "")]
    intent: String,
    #[arg(long, default_value_t = 16_384)]
    max_bytes: usize,
    #[arg(long, default_value = ".semantic-thread/agent-context.json")]
    evidence: PathBuf,
    #[arg(long)]
    output: Option<PathBuf>,
    #[arg(long, default_value_t = 2)]
    max_roots: usize,
    #[arg(long = "max-nodes", hide = true)]
    _max_nodes: Option<usize>,
}
#[derive(Args)]
struct TaskApplyArgs {
    #[arg(long)]
    repo: PathBuf,
    #[arg(long)]
    context: PathBuf,
    #[arg(long = "edit-plan")]
    edit_plan: PathBuf,
    #[arg(long)]
    target_ref: String,
    #[arg(long, default_value = "semantic-task-agent")]
    actor: String,
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
            // A task context is the immutable base of the following transaction,
            // so its snapshot must be published in the same compilation namespace
            // that task-apply and commit validate.
            let mut repository_index =
                RepositoryIndex::open_compilation(&repo, Some(&args.compilation))?;
            let index_snapshot = repository_index.update(&index_facts)?;
            let selection = task_context::select(&repo, &index_facts, &args.terms, &args.intent)?;
            let resolutions = selection
                .root_symbols(args.max_roots)
                .into_iter()
                .map(|symbol| {
                    worker.request(
                        RequestKind::ResolveSymbol,
                        &json!({"repo":repo,"compilation":args.compilation,"symbol":symbol}),
                    )
                })
                .collect::<Result<Vec<_>, _>>()?;
            let threads = resolutions
                .iter()
                .map(|resolution| {
                    build_task_thread(
                        worker,
                        &repo,
                        &args.compilation,
                        &project,
                        &index_snapshot,
                        resolution,
                    )
                })
                .collect::<Result<Vec<_>, _>>()?;
            let evidence_path = if args.evidence.is_absolute() {
                args.evidence
            } else {
                repo.join(args.evidence)
            };
            let base_revision = git_head(&repo)?;
            let (context, evidence) = task_context::build(task_context::TaskContextBuild {
                repo: &repo,
                terms: &args.terms,
                intent: &args.intent,
                compilation: &args.compilation,
                project: &project,
                index_facts: &index_facts,
                selection: &selection,
                resolutions: &resolutions,
                threads: &threads,
                base_revision: &base_revision,
                index_snapshot: &index_snapshot,
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
        Command::TaskApply(args) => with_worker(&workspace, |worker| {
            let repo = absolute(&args.repo)?;
            let evidence: Value = read_json(&args.context)?;
            if evidence["schema"] != "semantic-task-context-evidence/0.2" {
                return Err(SthreadError::new(
                    ErrorCode::InvalidInput,
                    "task-apply needs semantic-task-context-evidence/0.2",
                ));
            }
            let context = &evidence["context"];
            if context
                .pointer("/completeness/status")
                .and_then(Value::as_str)
                != Some("COMPLETE_TASK")
                || evidence
                    .pointer("/stdoutCompleteness/status")
                    .and_then(Value::as_str)
                    != Some("COMPLETE_TASK")
            {
                return Err(SthreadError::new(
                    ErrorCode::IncompleteSemanticAnalysis,
                    "task context or its bounded stdout projection is not COMPLETE_TASK; rebuild or inspect its boundaries",
                ));
            }
            let thread: ThreadIr = serde_json::from_value(
                evidence["threads"]
                    .as_array()
                    .and_then(|threads| threads.first())
                    .cloned()
                    .ok_or_else(|| {
                        SthreadError::new(ErrorCode::InvalidInput, "task context has no Thread IR")
                    })?,
            )
            .map_err(parse_error)?;
            let mut plan: Value = read_json(&args.edit_plan)?;
            expand_task_targets(&mut plan, context)?;
            let operations: Vec<EditOperation> = serde_json::from_value(
                plan["operations"]
                    .as_array()
                    .cloned()
                    .map(Value::Array)
                    .ok_or_else(|| {
                        SthreadError::new(
                            ErrorCode::InvalidInput,
                            "edit plan has no operations array",
                        )
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
            if base_revision != thread.snapshot.base_revision {
                return Err(SthreadError::new(
                    ErrorCode::PreconditionFailed,
                    "task context snapshot does not match its Thread IR",
                ));
            }
            let edit = EditIr {
                schema: "semantic-edit/0.1".into(),
                thread_id: thread.thread_id.clone(),
                base_revision: base_revision.clone(),
                operations,
                expected_write_set,
            };
            let test_tasks = context
                .pointer("/validationPlan/targetedArgs")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect::<Vec<_>>();
            let mut transaction = Transaction {
                schema: "semantic-transaction/0.1".into(),
                tx_id: format!("tx:{}", uuid::Uuid::new_v4()),
                actor_id: args.actor,
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
                edit,
                preview: None,
                expected_write_set_hash: None,
                actual_write_set_hash: None,
                validation_evidence: vec![json!({
                    "kind":"TASK_CONTEXT",
                    "contextHash":canonical::hash(context).map_err(parse_error)?,
                    "evidence":args.context
                })],
                test_tasks,
                candidate_commit: None,
                final_commit: None,
                target_ref: None,
            };
            let result = transaction::commit(&repo, &mut transaction, &args.target_ref, worker)?;
            if let Some(output) = args.output.as_deref() {
                write_artifact(output, &transaction)?;
                let build = transaction
                    .validation_evidence
                    .iter()
                    .find(|evidence| evidence["kind"] == "BUILD")
                    .cloned()
                    .unwrap_or(Value::Null);
                return Ok(json!({
                    "schema":"semantic-task-apply-receipt/0.1",
                    "status":transaction.status,
                    "finalCommit":transaction.final_commit,
                    "changedFiles":transaction.preview.as_ref().map(|preview| &preview.changed_files),
                    "build":build,
                    "transactionArtifact":output
                }));
            }
            Ok(
                json!({"schema":"semantic-task-apply/0.1","result":result,"transaction":transaction}),
            )
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

fn build_task_thread(
    worker: &mut WorkerClient,
    repo: &Path,
    compilation: &str,
    project: &Value,
    index_snapshot: &str,
    resolution: &Value,
) -> Result<ThreadIr, SthreadError> {
    let symbol = resolution
        .pointer("/declaration/legacySymbolId")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            SthreadError::new(
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
            SthreadError::new(
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
        .map_err(|error| SthreadError::new(ErrorCode::Internal, error.to_string()))
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

fn expand_task_targets(plan: &mut Value, context: &Value) -> Result<(), SthreadError> {
    let mut targets = std::collections::BTreeMap::<String, Value>::new();
    for key in ["editSurfaces", "contracts", "tests"] {
        for item in context[key].as_array().into_iter().flatten() {
            for target_key in ["declarationTarget", "bodyTarget"] {
                let Some(target) = item.get(target_key) else {
                    continue;
                };
                let Some(id) = target["anchorId"].as_str() else {
                    continue;
                };
                targets.insert(id.to_owned(), target.clone());
            }
        }
    }
    for operation in plan["operations"].as_array_mut().into_iter().flatten() {
        if operation["kind"] == "CREATE_FILE" {
            continue;
        }
        let Some(target_id) = operation
            .pointer("/target/targetId")
            .and_then(Value::as_str)
            .map(str::to_owned)
        else {
            return Err(SthreadError::new(
                ErrorCode::InvalidInput,
                "every non-CREATE_FILE task operation must reference a context targetId",
            ));
        };
        operation["target"] = targets.get(&target_id).cloned().ok_or_else(|| {
            SthreadError::new(
                ErrorCode::InvalidInput,
                format!("edit plan references unknown task target {target_id}"),
            )
        })?;
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
