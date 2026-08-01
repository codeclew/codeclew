use serde_json::Value;
use std::process::Command;
use sthread::worker::workspace_root;

#[test]
fn cli_builds_one_bounded_context_pack_with_edit_anchor_and_evidence() {
    let root = workspace_root();
    let fixture = root.join("fixtures/kotlin-2-1");
    let temporary = tempfile::tempdir().unwrap();
    let evidence = temporary.path().join("evidence.json");
    let output = Command::new(env!("CARGO_BIN_EXE_sthread"))
        .args([
            "agent-context",
            "--repo",
            fixture.to_str().unwrap(),
            "--term",
            "applyAdaptive",
            "--term",
            "Adaptive",
            "--max-bytes",
            "12288",
            "--evidence",
            evidence.to_str().unwrap(),
        ])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stdout.len() <= 12_288);
    let context: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(context["schema"], "semantic-agent-context-pack/0.1");
    let declaration = context["declarations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|declaration| declaration["name"] == "applyAdaptive")
        .unwrap();
    assert!(
        declaration["editAnchor"]["syntaxKind"]
            .as_str()
            .is_some_and(|kind| kind.starts_with("Kt"))
    );
    assert!(
        declaration["sourceText"]
            .as_str()
            .unwrap()
            .contains("applyAdaptive")
    );

    let stored: Value = serde_json::from_slice(&std::fs::read(evidence).unwrap()).unwrap();
    assert_eq!(stored["schema"], "semantic-agent-context-evidence/0.1");
    assert!(!stored["index"]["files"].as_array().unwrap().is_empty());
}
