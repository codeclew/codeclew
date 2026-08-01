use serde_json::json;
use std::time::Instant;
use sthread::canonical;
use sthread::graph;
use sthread::model::{LocalGraph, SlicePolicy, Snapshot};
use sthread::proto::RequestKind;
use sthread::worker::{WorkerClient, workspace_root};

fn main() {
    let root = workspace_root();
    let fixture = root.join("fixtures/kotlin-basic");
    let started = Instant::now();
    let mut worker = WorkerClient::start(&root).expect("worker startup");
    let worker_startup = started.elapsed().as_millis();

    let started = Instant::now();
    worker
        .request(
            RequestKind::ValidateCandidate,
            &json!({"file":"Probe.kt","source":"fun probe() = 1"}),
        )
        .expect("IPC/PSI probe");
    let ipc_plus_psi = started.elapsed().as_millis();

    worker
        .request(
            RequestKind::OpenProject,
            &json!({"repo":fixture,"compilation":":/main"}),
        )
        .expect("project snapshot setup");

    worker
        .request(
            RequestKind::IndexFiles,
            &json!({"repo":fixture,"compilation":":/main","syntaxOnly":true,"files":["src/main/kotlin/com/acme/Samples.kt"]}),
        )
        .expect("file index warmup");
    let started = Instant::now();
    worker
        .request(
            RequestKind::IndexFiles,
            &json!({"repo":fixture,"compilation":":/main","syntaxOnly":true,"files":["src/main/kotlin/com/acme/Samples.kt"]}),
        )
        .expect("warm file reindex");
    let warm_file_reindex = started.elapsed().as_millis();

    let source =
        std::fs::read_to_string(fixture.join("src/main/kotlin/com/acme/Samples.kt")).unwrap();
    let started = Instant::now();
    worker.request(RequestKind::ResolveExpression, &json!({"repo":fixture,"file":"src/main/kotlin/com/acme/Samples.kt","offset":source.find("value *= 2").unwrap()})).expect("anchor");
    let anchor_resolution = started.elapsed().as_millis();

    let started = Instant::now();
    worker
        .request(
            RequestKind::ResolveSymbol,
            &json!({"repo":fixture,"symbol":"com.acme.total"}),
        )
        .expect("K2 facts");
    let k2_analysis_cold = started.elapsed().as_millis();

    let started = Instant::now();
    worker
        .request(
            RequestKind::ResolveSymbol,
            &json!({"repo":fixture,"symbol":"com.acme.total"}),
        )
        .expect("warm K2 facts");
    let k2_analysis = started.elapsed().as_millis();

    let started = Instant::now();
    let raw = worker
        .request(
            RequestKind::BuildLocalGraph,
            &json!({"repo":fixture,"symbol":"com.acme.total"}),
        )
        .expect("FIR CFG");
    let fir_extraction = started.elapsed().as_millis();
    let local: LocalGraph = serde_json::from_value(raw).unwrap();

    let started = Instant::now();
    let graph = graph::enrich(local);
    let ssa_and_control = started.elapsed().as_millis();
    let seed = graph
        .nodes
        .iter()
        .find(|node| node.kind == "RETURN")
        .unwrap()
        .id
        .clone();

    let started = Instant::now();
    let thread = graph::slice(
        &graph,
        &seed,
        SlicePolicy::default(),
        Snapshot {
            base_revision: "benchmark".into(),
            project_model_hash: "benchmark".into(),
            compiler_version: "2.4.10".into(),
        },
        json!({"kind":"FUNCTION_RETURN","symbol":"com.acme.total","nodeId":seed}),
    )
    .unwrap();
    let slicing = started.elapsed().as_millis();

    let started = Instant::now();
    let _ = canonical::bytes(&thread).unwrap();
    let serialization = started.elapsed().as_millis();
    worker.shutdown().unwrap();
    println!("{}", serde_json::to_string(&json!({"workerStartup":worker_startup,"ipcPlusPsiParse":ipc_plus_psi,"warmFileReindex":warm_file_reindex,"anchorResolution":anchor_resolution,"k2AnalysisCold":k2_analysis_cold,"k2Analysis":k2_analysis,"firExtraction":fir_extraction,"ssaAndControl":ssa_and_control,"slicing":slicing,"canonicalSerialization":serialization})).unwrap());
}
