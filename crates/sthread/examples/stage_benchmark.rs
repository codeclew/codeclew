use serde_json::json;
use std::collections::BTreeMap;
use std::path::Path;
use std::process::Command;
use std::time::Instant;
use sthread::canonical;
use sthread::graph;
use sthread::model::{EditIr, EditOperation, LocalGraph, Replacement, SlicePolicy, Snapshot};
use sthread::proto::RequestKind;
use sthread::transaction;
use sthread::worker::{WorkerClient, workspace_root};

const SAMPLES: usize = 20;
const SOURCE_FILE: &str = "src/main/kotlin/com/acme/Samples.kt";

fn copy_fixture(from: &Path, to: &Path) {
    for entry in walkdir::WalkDir::new(from).into_iter().map(Result::unwrap) {
        let relative = entry.path().strip_prefix(from).unwrap();
        if relative.components().any(|part| {
            matches!(
                part.as_os_str().to_str(),
                Some(".gradle" | "build" | ".semantic-thread")
            )
        }) {
            continue;
        }
        let target = to.join(relative);
        if entry.file_type().is_dir() {
            std::fs::create_dir_all(&target).unwrap();
        } else {
            std::fs::copy(entry.path(), &target).unwrap();
        }
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let wrapper = to.join("gradlew");
        let mut permissions = std::fs::metadata(&wrapper).unwrap().permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(wrapper, permissions).unwrap();
    }
}

fn git(repo: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .args(args)
        .current_dir(repo)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().into()
}

fn millis(started: Instant) -> u64 {
    started.elapsed().as_micros().div_ceil(1000) as u64
}

fn micros(started: Instant) -> u64 {
    started.elapsed().as_micros() as u64
}

fn p95(samples: &[u64]) -> u64 {
    let mut sorted = samples.to_vec();
    sorted.sort_unstable();
    sorted[(sorted.len() * 95).div_ceil(100) - 1]
}

fn main() {
    let root = workspace_root();
    let temp = tempfile::tempdir().unwrap();
    let fixture = temp.path().join("repo");
    copy_fixture(&root.join("fixtures/kotlin-basic"), &fixture);
    git(&fixture, &["init", "-q", "-b", "main"]);
    git(&fixture, &["add", "."]);
    git(
        &fixture,
        &[
            "-c",
            "user.name=Benchmark",
            "-c",
            "user.email=benchmark@localhost",
            "commit",
            "-qm",
            "baseline",
        ],
    );
    let base = git(&fixture, &["rev-parse", "HEAD"]);
    let source_path = fixture.join(SOURCE_FILE);
    let original_source = std::fs::read_to_string(&source_path).unwrap();

    let started = Instant::now();
    let mut worker = WorkerClient::start(&root).expect("worker startup");
    let worker_startup = millis(started);
    let project = worker
        .request(
            RequestKind::OpenProject,
            &json!({"repo":fixture,"compilation":":/main"}),
        )
        .expect("clean project snapshot");

    let started = Instant::now();
    worker
        .request(
            RequestKind::IndexFiles,
            &json!({"repo":fixture,"compilation":":/main"}),
        )
        .expect("cold K2 semantic index");
    let cold_semantic_index = millis(started);

    let mut semantic_reindex_samples = Vec::new();
    for sample in 0..SAMPLES {
        std::fs::write(
            &source_path,
            format!("{original_source}\n// semantic reindex sample {sample}\n"),
        )
        .unwrap();
        let started = Instant::now();
        worker
            .request(
                RequestKind::IndexFiles,
                &json!({"repo":fixture,"compilation":":/main","files":[SOURCE_FILE]}),
            )
            .expect("changed-file K2 semantic reindex");
        semantic_reindex_samples.push(millis(started));
    }
    std::fs::write(&source_path, &original_source).unwrap();
    worker
        .request(
            RequestKind::IndexFiles,
            &json!({"repo":fixture,"compilation":":/main","files":[SOURCE_FILE]}),
        )
        .expect("restore semantic baseline");

    let mut ipc_serialization_samples = Vec::new();
    let mut psi_parse_samples = Vec::new();
    for sample in 0..SAMPLES {
        let started = Instant::now();
        let response = worker
            .request(
                RequestKind::ValidateCandidate,
                &json!({"file":"Probe.kt","source":format!("fun probe{sample}() = {sample}")}),
            )
            .expect("IPC/PSI probe");
        let total_micros = micros(started);
        let psi_micros = response["psiParseMicros"].as_u64().unwrap_or(0);
        psi_parse_samples.push(psi_micros);
        ipc_serialization_samples.push(total_micros.saturating_sub(psi_micros));
    }

    let offset = original_source.find("value *= 2").unwrap();
    let mut anchor_samples = Vec::new();
    for _ in 0..SAMPLES {
        let started = Instant::now();
        worker
            .request(
                RequestKind::ResolveExpression,
                &json!({"repo":fixture,"file":SOURCE_FILE,"offset":offset,"compilation":":/main"}),
            )
            .expect("anchor");
        anchor_samples.push(millis(started));
    }

    let mut resolve_samples = Vec::new();
    for _ in 0..SAMPLES {
        let started = Instant::now();
        worker
            .request(
                RequestKind::ResolveSymbol,
                &json!({"repo":fixture,"symbol":"com.acme.total","compilation":":/main"}),
            )
            .expect("resolve symbol");
        resolve_samples.push(millis(started));
    }

    let mut cfg_and_ssa_samples = Vec::new();
    let mut fir_extraction_samples = Vec::new();
    let mut rust_graph_samples = Vec::new();
    let mut ssa_samples = Vec::new();
    let mut latest_graph = None;
    for _ in 0..SAMPLES {
        let complete_started = Instant::now();
        let fir_started = Instant::now();
        let raw = worker
            .request(
                RequestKind::BuildLocalGraph,
                &json!({"repo":fixture,"symbol":"com.acme.total","compilation":":/main"}),
            )
            .expect("FIR CFG");
        fir_extraction_samples.push(millis(fir_started));
        let local: LocalGraph = serde_json::from_value(raw).unwrap();
        let (enriched, profile) = graph::enrich_profiled(local);
        rust_graph_samples.push(profile.rust_graph_construction_micros);
        ssa_samples.push(profile.ssa_and_control_micros);
        cfg_and_ssa_samples.push(millis(complete_started));
        latest_graph = Some(enriched);
    }
    let graph = latest_graph.unwrap();
    let seed = graph
        .nodes
        .iter()
        .find(|node| node.kind == "RETURN")
        .unwrap()
        .id
        .clone();
    let snapshot = Snapshot {
        base_revision: base.clone(),
        project_model_hash: project["projectModelHash"].as_str().unwrap().into(),
        compiler_version: "2.4.10".into(),
        compilation: ":/main".into(),
        compile_task: project["compileTask"].as_str().unwrap().into(),
        test_tasks: vec![],
        ..Snapshot::default()
    };
    let mut extraction_samples = Vec::new();
    let mut latest_thread = None;
    for _ in 0..SAMPLES {
        let started = Instant::now();
        latest_thread = Some(
            graph::slice(
                &graph,
                &seed,
                SlicePolicy::default(),
                snapshot.clone(),
                json!({"kind":"FUNCTION_RETURN","symbol":graph.symbol,"nodeId":seed}),
            )
            .unwrap(),
        );
        extraction_samples.push(millis(started));
    }
    let thread = latest_thread.unwrap();
    let mut serialization_samples = Vec::new();
    for _ in 0..SAMPLES {
        let started = Instant::now();
        let _ = canonical::bytes(&thread).unwrap();
        serialization_samples.push(millis(started));
    }

    let target = thread
        .nodes
        .iter()
        .find_map(|node| {
            node.origin
                .as_ref()
                .filter(|origin| origin["sourceText"] == "value *= 2")
                .cloned()
        })
        .unwrap();
    let mut edit_preview_samples = Vec::new();
    for sample in 0..SAMPLES {
        let edit = EditIr {
            schema: "semantic-edit/0.1".into(),
            thread_id: thread.thread_id.clone(),
            base_revision: base.clone(),
            operations: vec![EditOperation {
                op_id: format!("op:benchmark:{sample}"),
                kind: "REPLACE_EXPRESSION".into(),
                target: target.clone(),
                replacement: Replacement {
                    kotlin: format!("value = value + value /* preview sample {sample} */"),
                },
                preconditions: BTreeMap::new(),
                postconditions: BTreeMap::new(),
            }],
            expected_write_set: vec![],
        };
        let started = Instant::now();
        transaction::preview(&fixture, &thread, &edit, &mut worker)
            .expect("unique semantic edit preview");
        edit_preview_samples.push(millis(started));
    }
    worker.shutdown().unwrap();

    let warm_semantic_reindex = p95(&semantic_reindex_samples);
    let anchor = p95(&anchor_samples);
    let resolve = p95(&resolve_samples);
    let cfg_and_ssa = p95(&cfg_and_ssa_samples);
    let fir_extraction = p95(&fir_extraction_samples);
    let rust_graph = p95(&rust_graph_samples);
    let ssa = p95(&ssa_samples);
    let extraction = p95(&extraction_samples);
    let serialization = p95(&serialization_samples);
    let edit_preview = p95(&edit_preview_samples);
    println!(
        "{}",
        serde_json::to_string(&json!({
            "sampleCount":SAMPLES,
            "workerStartup":worker_startup,
            "coldSemanticIndex":cold_semantic_index,
            "ipcAndProtocolSerializationMicrosP95":p95(&ipc_serialization_samples),
            "psiParseMicrosP95":p95(&psi_parse_samples),
            "warmSemanticFileReindexP95":warm_semantic_reindex,
            "k2SemanticAnalysisCold":cold_semantic_index,
            "k2ChangedFileAnalysisP95":warm_semantic_reindex,
            "anchorResolutionP95":anchor,
            "resolveSymbolP95":resolve,
            "firCfgExtractionP95":fir_extraction,
            "rustGraphConstructionMicrosP95":rust_graph,
            "ssaAndControlMicrosP95":ssa,
            "localCfgAndSsaP95":cfg_and_ssa,
            "localThreadExtractionP95":extraction,
            "boundedSliceP95":extraction,
            "canonicalSerializationP95":serialization,
            "firstEditPreview":edit_preview_samples[0],
            "editPreviewP95":edit_preview,
            "sloPassed":{
                "warmSemanticFileReindex":warm_semantic_reindex <= 300,
                "anchorResolution":anchor <= 100,
                "resolveSymbol":resolve <= 150,
                "localCfgAndSsa":cfg_and_ssa <= 500,
                "localThreadExtraction":extraction <= 800,
                "boundedSlice":extraction <= 2000,
                "canonicalSerialization":serialization <= 100,
                "editPreview":edit_preview <= 700
            }
        }))
        .unwrap()
    );
}
