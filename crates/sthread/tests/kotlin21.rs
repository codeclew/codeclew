use serde_json::json;
use sthread::proto::RequestKind;
use sthread::worker::{WorkerClient, workspace_root};

#[test]
fn selects_matching_kotlin_21_worker_and_resolves_extension_names() {
    let root = workspace_root();
    let fixture = root.join("fixtures/kotlin-2-1");
    let mut worker = WorkerClient::start(&root).unwrap();

    let project = worker
        .request(
            RequestKind::OpenProject,
            &json!({"repo":fixture,"compilation":":/main"}),
        )
        .unwrap();
    assert_eq!(project["compilerVersion"], "2.1.21");
    assert_eq!(project["workerCompilerVersion"], "2.1.21");
    assert_eq!(project["languageVersion"], "2.1");
    assert_eq!(project["apiVersion"], "2.1");
    assert_eq!(worker.capabilities.compiler_version, "2.1.21");

    let index = worker
        .request(
            RequestKind::IndexFiles,
            &json!({"repo":fixture,"compilation":":/main"}),
        )
        .unwrap();
    assert_eq!(index["compilerVersion"], "2.1.21");
    assert_eq!(index["k2Validated"], true);
    assert!(
        index["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .all(|diagnostic| {
                !diagnostic["message"]
                    .as_str()
                    .unwrap_or_default()
                    .contains("IncompatibleClassChangeError")
            })
    );

    for symbol in ["applyAdaptive", "com.acme.applyAdaptive"] {
        let resolved = worker
            .request(
                RequestKind::ResolveSymbol,
                &json!({"repo":fixture,"compilation":":/main","symbol":symbol}),
            )
            .unwrap();
        assert_eq!(resolved["declaration"]["name"], "applyAdaptive");
        assert_eq!(resolved["k2Validated"], true);
    }

    let kotlin24 = root.join("fixtures/kotlin-basic");
    let project24 = worker
        .request(
            RequestKind::OpenProject,
            &json!({"repo":kotlin24,"compilation":":/main"}),
        )
        .unwrap();
    assert_eq!(project24["compilerVersion"], "2.4.10");
    assert_eq!(project24["workerCompilerVersion"], "2.4.10");
    assert_eq!(worker.capabilities.compiler_version, "2.4.10");

    worker.shutdown().unwrap();
}
