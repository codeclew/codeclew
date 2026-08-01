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
            "--intent",
            "Apply adaptive settings to the selected Kotlin function and preserve its typed contract",
            "--max-bytes",
            "16384",
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
    assert!(output.stdout.len() <= 16_384);
    let context: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(context["schema"], "semantic-task-context/0.2");
    let declaration = context["editSurfaces"]
        .as_array()
        .unwrap()
        .iter()
        .find(|declaration| declaration["name"] == "applyAdaptive")
        .unwrap();
    assert!(declaration["bodyTargetId"].as_str().is_some());
    assert!(
        declaration["sourceText"]
            .as_str()
            .unwrap()
            .contains("applyAdaptive")
    );
    assert!(context.get("references").is_none());
    assert_eq!(context["editPlan"]["schema"], "semantic-task-edit-plan/0.1");
    assert!(context["editPlan"].get("recommendedRecipe").is_none());
    assert!(
        context["editPlan"]["instruction"]
            .as_str()
            .unwrap()
            .contains("same-target rewrites merge")
    );
    assert!(
        declaration["declarationTargetId"]
            .as_str()
            .is_some_and(|target| target.starts_with('S'))
    );
    let stored: Value = serde_json::from_slice(&std::fs::read(evidence).unwrap()).unwrap();
    assert_eq!(stored["schema"], "semantic-task-context-evidence/0.2");
    assert_eq!(stored["stdoutCompleteness"]["status"], "COMPLETE_TASK");
    assert!(!stored["index"]["files"].as_array().unwrap().is_empty());
    assert!(!stored["threads"].as_array().unwrap().is_empty());
    let stored_declaration = stored["context"]["editSurfaces"]
        .as_array()
        .unwrap()
        .iter()
        .find(|declaration| declaration["name"] == "applyAdaptive")
        .unwrap();
    assert!(
        stored_declaration["bodyTarget"]["syntaxKind"]
            .as_str()
            .is_some_and(|kind| kind.starts_with("Kt"))
    );
}
