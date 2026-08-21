mod support;

use serde_json::Value;
use std::io::Write;
use std::process::{Command, Stdio};

#[test]
fn semanticd_is_long_lived_and_does_not_claim_project_native_cache_hits() {
    let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("fixtures/kotlin-basic")
        .canonicalize()
        .unwrap();
    support::seed_build_caches(&fixture);
    let mut child = Command::new(env!("CARGO_BIN_EXE_semanticd"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let requests = format!(
        "{}\n{}\n{}\n{}\n{}\n{}\n{}\n",
        serde_json::json!({"id":1,"method":"health"}),
        serde_json::json!({"id":2,"method":"project.inspect","params":{"repo":fixture,"compilation":":/main"}}),
        serde_json::json!({"id":3,"method":"project.inspect","params":{"repo":fixture,"compilation":":/main"}}),
        serde_json::json!({"id":4,"method":"project.inspect","params":{"repo":"/definitely/missing/semantic-thread-cache-test"}}),
        serde_json::json!({"id":5,"method":"project.inspect","params":{"repo":"/definitely/missing/semantic-thread-cache-test"}}),
        serde_json::json!({"id":6,"method":"metrics"}),
        serde_json::json!({"id":7,"method":"shutdown"}),
    );
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(requests.as_bytes())
        .unwrap();
    drop(child.stdin.take());
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success());
    let responses: Vec<Value> = String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(responses.len(), 7);
    assert_eq!(responses[0]["result"]["status"], "OK");
    assert!(!responses[3]["error"].is_null());
    assert!(!responses[4]["error"].is_null());
    let metrics = &responses[5]["result"]["metrics"];
    for required in [
        "request_duration_ms_total",
        "worker_startup_duration_ms",
        "worker_memory_bytes",
        "cache_hits",
        "files_parsed",
        "semantic_facts_extracted",
        "cfg_nodes",
        "slice_nodes",
        "slice_boundary_count",
        "anchor_resolution_attempts",
        "gradle_validation_duration_ms",
        "orphan_worktrees",
    ] {
        assert!(!metrics[required].is_null(), "missing metric {required}");
    }
    assert_eq!(metrics["cache_requests"], 2);
    assert_eq!(metrics["cache_hits"], 0);
    assert_eq!(responses[5]["result"]["cacheHitRate"], 0.0);
}
