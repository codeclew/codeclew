mod support;

use clew::worker::workspace_root;
use serde_json::Value;
use std::process::Command;
use std::sync::Mutex;

static CLI_LOCK: Mutex<()> = Mutex::new(());

#[test]
fn kotlin_21_cli_returns_a_bounded_l5_claim_with_exact_l0_trace_and_explicit_boundary() {
    let _guard = CLI_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let root = workspace_root();
    let fixture = root.join("fixtures/kotlin-2-1");
    support::seed_build_caches(&fixture);
    let output = Command::new(env!("CARGO_BIN_EXE_clew"))
        .args([
            "projection",
            "--repo",
            fixture.to_str().unwrap(),
            "--compilation",
            ":/main",
            "--symbol",
            "com.acme.applyAdaptive",
            "--level",
            "l5",
            "--thread",
            "data",
            "--claim",
            "trace adaptive behavior",
            "--max-bytes",
            "32768",
        ])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stdout.len() <= 32_768);
    let result: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["schema"], "semantic-projection/0.1");
    assert_eq!(result["status"], "PARTIAL_BOUNDARY");
    assert!(result["boundaries"].as_array().is_some_and(|boundaries| {
        boundaries.iter().any(|boundary| {
            boundary["id"] == "THREAD_COMPLETENESS"
                && boundary["state"] == "UNSUPPORTED"
                && boundary["reason"]
                    .as_str()
                    .is_some_and(|reason| reason.starts_with("sha256:"))
        })
    }));
    assert_eq!(result["provenance"]["compilerVersion"], "2.1.21");
    assert_eq!(result["nodes"].as_array().unwrap().len(), 1);
    assert_eq!(result["nodes"][0]["level"], "L5");
    assert!(result["nodes"][0].get("source").is_none());
    assert!(
        result["nodes"][0]["evidence"]
            .as_array()
            .is_some_and(|evidence| !evidence.is_empty())
    );
    assert!(
        result["nodes"][0]["evidence"]
            .as_array()
            .unwrap()
            .iter()
            .all(|link| link["path"].as_array().is_some_and(|path| path.len() == 6))
    );
}

#[test]
fn kotlin_21_cli_refuses_a_fabricated_thread_kind_and_an_unrenderable_budget() {
    let _guard = CLI_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let root = workspace_root();
    let fixture = root.join("fixtures/kotlin-2-1");
    support::seed_build_caches(&fixture);
    let base = [
        "projection",
        "--repo",
        fixture.to_str().unwrap(),
        "--compilation",
        ":/main",
        "--symbol",
        "com.acme.applyAdaptive",
        "--level",
        "l5",
        "--claim",
        "trace adaptive behavior",
    ];

    let fabricated = Command::new(env!("CARGO_BIN_EXE_clew"))
        .args(base)
        .args(["--thread", "config"])
        .output()
        .unwrap();
    assert!(!fabricated.status.success());
    let fabricated_error: Value = serde_json::from_slice(&fabricated.stdout).unwrap();
    assert_eq!(
        fabricated_error["error"]["code"],
        "INCOMPLETE_SEMANTIC_ANALYSIS"
    );
    assert!(
        fabricated_error["error"]["message"]
            .as_str()
            .unwrap()
            .contains("no Config evidence")
    );

    let too_small = Command::new(env!("CARGO_BIN_EXE_clew"))
        .args(base)
        .args(["--thread", "data", "--max-bytes", "2000"])
        .output()
        .unwrap();
    assert!(!too_small.status.success());
    assert!(too_small.stdout.len() <= 2_000);
    let budget_error: Value = serde_json::from_slice(&too_small.stdout).unwrap();
    assert_eq!(budget_error["error"]["code"], "SLICE_BUDGET_EXCEEDED");

    let no_envelope_budget = Command::new(env!("CARGO_BIN_EXE_clew"))
        .args(base)
        .args(["--thread", "data", "--max-bytes", "1"])
        .output()
        .unwrap();
    assert!(!no_envelope_budget.status.success());
    assert!(no_envelope_budget.stdout.is_empty());
}
