use serde_json::Value;
use std::io::Write;
use std::process::{Command, Stdio};

#[test]
fn semanticd_is_long_lived_and_exports_required_metrics() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_semanticd"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(
            b"{\"id\":1,\"method\":\"health\"}\n{\"id\":2,\"method\":\"metrics\"}\n{\"id\":3,\"method\":\"shutdown\"}\n",
        )
        .unwrap();
    drop(child.stdin.take());
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success());
    let responses: Vec<Value> = String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(responses.len(), 3);
    assert_eq!(responses[0]["result"]["status"], "OK");
    let metrics = &responses[1]["result"]["metrics"];
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
}
