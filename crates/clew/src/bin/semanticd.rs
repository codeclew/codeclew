use clew::error::{ClewError, ErrorCode};
use clew::graph;
use clew::index::{REPOSITORY_INDEX_FACT, RepositoryIndex};
use clew::model::{EditIr, LocalGraph, SlicePolicy, Snapshot, ThreadIr, Transaction};
use clew::proto::RequestKind;
use clew::transaction;
use clew::worker::{WorkerClient, workspace_root};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::io::{self, BufRead, Write};
use std::path::Path;
use std::time::Instant;

#[derive(Default)]
struct Metrics {
    values: BTreeMap<String, u64>,
    failures: BTreeMap<String, u64>,
    conflicts: BTreeMap<String, u64>,
}

impl Metrics {
    fn add(&mut self, name: &str, value: u64) {
        *self.values.entry(name.into()).or_default() += value;
    }

    fn set(&mut self, name: &str, value: u64) {
        self.values.insert(name.into(), value);
    }

    fn snapshot(&self, worker_pid: u32) -> Value {
        let mut values = self.values.clone();
        values.insert("worker_memory_bytes".into(), worker_memory(worker_pid));
        for required in [
            "request_duration_ms_total",
            "request_count",
            "worker_startup_duration_ms",
            "worker_memory_bytes",
            "cache_hits",
            "cache_requests",
            "files_parsed",
            "semantic_facts_extracted",
            "cfg_nodes",
            "slice_nodes",
            "slice_boundary_count",
            "anchor_resolution_attempts",
            "gradle_validation_duration_ms",
            "orphan_worktrees",
        ] {
            values.entry(required.into()).or_default();
        }
        json!({
            "schema":"semantic-metrics/0.1",
            "metrics":values,
            "validationFailuresByCategory":self.failures,
            "transactionConflictsByCategory":self.conflicts,
            "cacheHitRate": if values.get("cache_requests").copied().unwrap_or(0) == 0 { 0.0 } else { values.get("cache_hits").copied().unwrap_or(0) as f64 / values["cache_requests"] as f64 }
        })
    }
}

fn main() {
    let startup = Instant::now();
    let mut worker = match WorkerClient::start(&workspace_root()) {
        Ok(worker) => worker,
        Err(error) => {
            println!(
                "{}",
                serde_json::to_string(&json!({"schema":"semanticd-response/0.1","error":error}))
                    .unwrap()
            );
            std::process::exit(1);
        }
    };
    let mut metrics = Metrics::default();
    metrics.set(
        "worker_startup_duration_ms",
        startup.elapsed().as_millis() as u64,
    );
    let stdin = io::stdin();
    let mut stdout = io::stdout().lock();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let started = Instant::now();
        let request: Result<Value, _> = serde_json::from_str(&line);
        let (id, method, result) = match request {
            Ok(request) => {
                let id = request.get("id").cloned().unwrap_or(Value::Null);
                let method = request
                    .get("method")
                    .and_then(Value::as_str)
                    .unwrap_or("<missing>")
                    .to_owned();
                let params = request.get("params").cloned().unwrap_or_else(|| json!({}));
                let result = dispatch(&method, &params, &mut worker, &mut metrics);
                (id, method, result)
            }
            Err(error) => (
                Value::Null,
                "<decode>".into(),
                Err(ClewError::new(ErrorCode::InvalidInput, error.to_string())),
            ),
        };
        let elapsed = started.elapsed().as_millis() as u64;
        metrics.add("request_duration_ms_total", elapsed);
        metrics.add("request_count", 1);
        if let Err(error) = &result {
            *metrics
                .failures
                .entry(format!("{:?}", error.code))
                .or_default() += 1;
            if matches!(
                error.code,
                ErrorCode::RwConflict
                    | ErrorCode::WwConflict
                    | ErrorCode::StaleRequiresReslice
                    | ErrorCode::RefCompareAndSwapFailed
            ) {
                *metrics
                    .conflicts
                    .entry(format!("{:?}", error.code))
                    .or_default() += 1;
            }
        }
        eprintln!(
            "{}",
            serde_json::to_string(&json!({"event":"semanticd_request","id":id,"method":method,"durationMs":elapsed,"success":result.is_ok()})).unwrap()
        );
        let response = match result {
            Ok(result) => json!({"schema":"semanticd-response/0.1","id":id,"result":result}),
            Err(error) => json!({"schema":"semanticd-response/0.1","id":id,"error":error}),
        };
        let _ = writeln!(stdout, "{}", serde_json::to_string(&response).unwrap());
        let _ = stdout.flush();
        if method == "shutdown" {
            break;
        }
    }
    let _ = worker.shutdown();
}

fn dispatch(
    method: &str,
    params: &Value,
    worker: &mut WorkerClient,
    metrics: &mut Metrics,
) -> Result<Value, ClewError> {
    let repo = || {
        params
            .get("repo")
            .and_then(Value::as_str)
            .unwrap_or_default()
    };
    let compilation = || {
        params
            .get("compilation")
            .and_then(Value::as_str)
            .unwrap_or(":/main")
    };
    if !repo().is_empty() {
        metrics.set("orphan_worktrees", orphan_worktrees(Path::new(repo())));
    }
    match method {
        "health" => Ok(json!({"status":"OK","service":"semanticd","workerPid":worker.pid()})),
        "metrics" => Ok(metrics.snapshot(worker.pid())),
        "project.inspect" => {
            profiled_worker_request(worker, RequestKind::OpenProject, params, metrics)
        }
        "index" => {
            let verified_facts = worker.index_files_verified(params)?;
            metrics.add("cache_requests", worker.last_profile.cache_requests);
            metrics.add("cache_hits", worker.last_profile.cache_hits);
            let facts = worker.inspect_verified_index(&verified_facts)?;
            let mut index =
                RepositoryIndex::open_compilation(Path::new(repo()), Some(compilation()))?;
            let snapshot = index.update_verified(&verified_facts, worker)?;
            let freshness = index.freshness_status(REPOSITORY_INDEX_FACT)?;
            let files = facts["files"].as_array().map_or(0, Vec::len) as u64;
            let semantic_facts = facts["files"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|file| file["semanticFacts"].as_array().map_or(0, Vec::len) as u64)
                .sum();
            metrics.add("files_parsed", files);
            metrics.add("semantic_facts_extracted", semantic_facts);
            Ok(
                json!({"indexSnapshot":snapshot,"invalidations":index.invalidations()?,"freshness":freshness,"facts":facts}),
            )
        }
        "resolve.symbol" => {
            profiled_worker_request(worker, RequestKind::ResolveSymbol, params, metrics)
        }
        "resolve.expression" => {
            metrics.add("anchor_resolution_attempts", 1);
            profiled_worker_request(worker, RequestKind::ResolveExpression, params, metrics)
        }
        "graph.local" => {
            let raw =
                profiled_worker_request(worker, RequestKind::BuildLocalGraph, params, metrics)?;
            let graph =
                graph::enrich(serde_json::from_value::<LocalGraph>(raw).map_err(parse_error)?);
            metrics.add("cfg_nodes", graph.nodes.len() as u64);
            serde_json::to_value(graph).map_err(parse_error)
        }
        "thread.slice" => {
            let graph: LocalGraph =
                serde_json::from_value(params["graph"].clone()).map_err(parse_error)?;
            let policy: SlicePolicy =
                serde_json::from_value(params["policy"].clone()).unwrap_or_default();
            let snapshot: Snapshot =
                serde_json::from_value(params["snapshot"].clone()).map_err(parse_error)?;
            let seed_id = params["seedId"].as_str().ok_or_else(|| {
                ClewError::new(ErrorCode::InvalidInput, "thread.slice needs seedId")
            })?;
            let thread = graph::slice(&graph, seed_id, policy, snapshot, params["seed"].clone())
                .map_err(parse_error)?;
            metrics.add("slice_nodes", thread.nodes.len() as u64);
            metrics.add(
                "slice_boundary_count",
                thread.completeness.boundaries.len() as u64,
            );
            serde_json::to_value(thread).map_err(parse_error)
        }
        "edit.preview" => {
            let thread: ThreadIr =
                serde_json::from_value(params["thread"].clone()).map_err(parse_error)?;
            let edit: EditIr =
                serde_json::from_value(params["edit"].clone()).map_err(parse_error)?;
            serde_json::to_value(transaction::preview(
                Path::new(repo()),
                &thread,
                &edit,
                worker,
            )?)
            .map_err(parse_error)
        }
        "tx.commit" => {
            let mut transaction: Transaction =
                serde_json::from_value(params["transaction"].clone()).map_err(parse_error)?;
            let transaction_id = transaction.tx_id.clone();
            let snapshot_id = transaction.thread.snapshot.index_snapshot.clone();
            let relevant = transaction
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
            let target = params["targetRef"].as_str().ok_or_else(|| {
                with_transaction_context(
                    ClewError::new(ErrorCode::InvalidInput, "tx.commit needs targetRef"),
                    &transaction_id,
                    &snapshot_id,
                    &relevant,
                )
            })?;
            let result =
                match transaction::commit(Path::new(repo()), &mut transaction, target, worker) {
                    Ok(result) => result,
                    Err(error) => {
                        metrics.add(
                            "gradle_validation_duration_ms",
                            gradle_duration_from_evidence(&error),
                        );
                        return Err(with_transaction_context(
                            error,
                            &transaction_id,
                            &snapshot_id,
                            &relevant,
                        ));
                    }
                };
            metrics.add(
                "gradle_validation_duration_ms",
                result["gradleValidationDurationMs"].as_u64().unwrap_or(0),
            );
            Ok(result)
        }
        "tx.inspect" => transaction::ledger(Path::new(repo()))?
            .inspect(params["transactionId"].as_str().unwrap_or_default()),
        "shutdown" => Ok(json!({"status":"SHUTTING_DOWN"})),
        _ => Err(ClewError::new(
            ErrorCode::InvalidInput,
            format!("unknown semanticd method {method}"),
        )),
    }
}

fn with_transaction_context(
    error: ClewError,
    transaction_id: &str,
    snapshot_id: &str,
    relevant: &str,
) -> ClewError {
    error
        .with_transaction(transaction_id)
        .with_snapshot(snapshot_id)
        .with_relevant(relevant)
}

fn profiled_worker_request(
    worker: &mut WorkerClient,
    kind: RequestKind,
    params: &Value,
    metrics: &mut Metrics,
) -> Result<Value, ClewError> {
    let result = worker.request(kind, params)?;
    metrics.add("cache_requests", worker.last_profile.cache_requests);
    metrics.add("cache_hits", worker.last_profile.cache_hits);
    Ok(result)
}

fn gradle_duration_from_evidence(error: &ClewError) -> u64 {
    error
        .evidence
        .iter()
        .filter_map(|item| {
            item.strip_prefix("gradleCompileDurationMs=")
                .or_else(|| item.strip_prefix("gradleTestDurationMs="))
                .and_then(|value| value.parse::<u64>().ok())
        })
        .sum()
}

fn orphan_worktrees(repo: &Path) -> u64 {
    let output = match std::process::Command::new("git")
        .args(["worktree", "list", "--porcelain"])
        .current_dir(repo)
        .output()
    {
        Ok(output) if output.status.success() => output,
        _ => return 0,
    };
    let text = String::from_utf8_lossy(&output.stdout);
    let mut orphaned = 0u64;
    let mut current_path: Option<&str> = None;
    for line in text.lines() {
        if let Some(path) = line.strip_prefix("worktree ") {
            if current_path.is_some_and(|path| !Path::new(path).exists()) {
                orphaned += 1;
            }
            current_path = Some(path);
        } else if line.starts_with("prunable ") {
            orphaned += 1;
            current_path = None;
        }
    }
    if current_path.is_some_and(|path| !Path::new(path).exists()) {
        orphaned += 1;
    }
    orphaned
}

fn worker_memory(pid: u32) -> u64 {
    std::process::Command::new("ps")
        .args(["-o", "rss=", "-p", &pid.to_string()])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .and_then(|value| value.trim().parse::<u64>().ok())
        .unwrap_or(0)
        * 1024
}

fn parse_error(error: impl std::fmt::Display) -> ClewError {
    ClewError::new(ErrorCode::WorkerProtocolMismatch, error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn early_commit_errors_keep_transaction_context() {
        let error = with_transaction_context(
            ClewError::new(ErrorCode::InvalidInput, "tx.commit needs targetRef"),
            "tx:context",
            "sha256:snapshot",
            "anchor:target",
        );
        assert_eq!(error.transaction_id.as_deref(), Some("tx:context"));
        assert_eq!(error.snapshot_id.as_deref(), Some("sha256:snapshot"));
        assert_eq!(&*error.relevant_anchors_or_symbols, &["anchor:target"]);
    }
}
