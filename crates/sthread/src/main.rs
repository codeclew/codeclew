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
            let mut resolutions = selection
                .root_symbols(1)
                .into_iter()
                .map(|symbol| {
                    worker.request(
                        RequestKind::ResolveSymbol,
                        &json!({"repo":repo,"compilation":args.compilation,"symbol":symbol}),
                    )
                })
                .collect::<Result<Vec<_>, _>>()?;
            while resolutions.len() < args.max_roots {
                let followups = selection.followup_symbols(
                    &resolutions,
                    args.max_roots.saturating_sub(resolutions.len()),
                );
                if followups.is_empty() {
                    break;
                }
                for symbol in followups {
                    resolutions.push(worker.request(
                        RequestKind::ResolveSymbol,
                        &json!({"repo":repo,"compilation":args.compilation,"symbol":symbol}),
                    )?);
                }
            }
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
            sthread::task_plan::expand_transient_transform(&mut plan, context, &evidence)?;
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
            );
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
fn normalize_task_plan(plan: &mut Value) -> Result<(), SthreadError> {
    if plan.get("operations").is_none()
        && let Some(edits) = plan.as_object_mut().and_then(|plan| plan.remove("edits"))
    {
        plan["operations"] = edits;
    }
    let operations = plan["operations"].as_array_mut().ok_or_else(|| {
        SthreadError::new(ErrorCode::InvalidInput, "edit plan has no operations array")
    })?;
    for (index, operation) in operations.iter_mut().enumerate() {
        let object = operation.as_object_mut().ok_or_else(|| {
            SthreadError::new(ErrorCode::InvalidInput, "task operation must be an object")
        })?;
        object
            .entry("opId")
            .or_insert_with(|| json!(format!("task-op-{}", index + 1)));
        if !object.contains_key("kind") {
            let kind = object.remove("op").ok_or_else(|| {
                SthreadError::new(ErrorCode::InvalidInput, "task operation must contain kind")
            })?;
            object.insert("kind".to_owned(), kind);
        }
        if object.get("kind").and_then(Value::as_str) == Some("CREATE_FILE")
            && !object.contains_key("replacement")
        {
            let lines = object
                .remove("kotlinLines")
                .or_else(|| object.remove("newLines"))
                .ok_or_else(|| {
                    SthreadError::new(
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
                    SthreadError::new(ErrorCode::InvalidInput, "CREATE_FILE needs a path")
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
                return Err(SthreadError::new(
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
                    SthreadError::new(
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
fn join_plan_lines(
    object: &mut serde_json::Map<String, Value>,
    lines_key: &str,
    text_key: &str,
) -> Result<bool, SthreadError> {
    let Some(lines) = object.remove(lines_key) else {
        return Ok(false);
    };
    if object.contains_key(text_key) {
        return Err(SthreadError::new(
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
                SthreadError::new(
                    ErrorCode::InvalidInput,
                    format!("task edit field {lines_key} must be a string or array of strings"),
                )
            })?
            .iter()
            .map(Value::as_str)
            .collect::<Option<Vec<_>>>()
            .ok_or_else(|| {
                SthreadError::new(
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
fn inject_created_contract_overrides(plan: &mut Value) -> Result<(), SthreadError> {
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
                SthreadError::new(
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
fn inject_created_type_imports(plan: &mut Value) -> Result<(), SthreadError> {
    let operations = plan["operations"].as_array().ok_or_else(|| {
        SthreadError::new(ErrorCode::InvalidInput, "edit plan has no operations array")
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

fn inject_explicit_target_imports(plan: &mut Value, context: &Value) -> Result<(), SthreadError> {
    let operations = plan["operations"].as_array().ok_or_else(|| {
        SthreadError::new(ErrorCode::InvalidInput, "edit plan has no operations array")
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

fn include_created_tests(test_tasks: &mut Vec<String>, plan: &Value, build_system: &str) {
    if build_system != "MAVEN"
        && test_tasks.iter().any(|argument| argument == "--tests")
        && !test_tasks.iter().any(|argument| argument == "test")
    {
        let position = test_tasks
            .iter()
            .position(|argument| argument == "cleanTest")
            .map_or(0, |index| index + 1);
        test_tasks.insert(position, "test".into());
    }
    let stems = plan["operations"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|operation| operation["kind"] == "CREATE_FILE")
        .filter_map(|operation| operation.pointer("/target/fileId").and_then(Value::as_str))
        .filter(|path| path.contains("/src/test/") || path.starts_with("src/test/"))
        .filter_map(|path| Path::new(path).file_stem()?.to_str())
        .map(str::to_owned)
        .collect::<std::collections::BTreeSet<_>>();
    if stems.is_empty() {
        return;
    }
    if build_system == "MAVEN" {
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
        return;
    }
    for stem in stems {
        let selector = format!("*{stem}");
        if test_tasks.iter().any(|argument| argument == &selector) {
            continue;
        }
        test_tasks.push("--tests".into());
        test_tasks.push(selector);
    }
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

#[cfg(test)]
mod task_plan_tests {
    use super::*;

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
            "target":{"fileId":"src/test/kotlin/com/acme/RunnerTest.kt"}
        }]});
        let mut gradle = vec!["cleanTest".into(), "--tests".into(), "*ExistingTest".into()];
        include_created_tests(&mut gradle, &plan, "GRADLE");
        assert_eq!(
            gradle,
            vec![
                "cleanTest",
                "test",
                "--tests",
                "*ExistingTest",
                "--tests",
                "*RunnerTest"
            ]
        );

        let mut maven = vec!["-Dtest=ExistingTest".into(), "test".into()];
        include_created_tests(&mut maven, &plan, "MAVEN");
        assert_eq!(maven, vec!["-Dtest=ExistingTest,RunnerTest", "test"]);
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
