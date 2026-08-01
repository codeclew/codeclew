use serde_json::json;
use std::path::Path;
use std::process::Command;
use sthread::canonical;
use sthread::error::ErrorCode;
use sthread::graph;
use sthread::model::{LocalGraph, SlicePolicy, Snapshot, ThreadIr};
use sthread::proto::RequestKind;
use sthread::worker::{WorkerClient, workspace_root};

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

#[test]
fn anchors_ir_and_candidate_validation_are_metamorphic() {
    let root = workspace_root();
    let temp = tempfile::tempdir().unwrap();
    let repo = temp.path().join("repo");
    copy_fixture(&root.join("fixtures/kotlin-basic"), &repo);
    Command::new("git")
        .args(["init", "-q", "-b", "main"])
        .current_dir(&repo)
        .status()
        .unwrap();
    Command::new("git")
        .args(["add", "."])
        .current_dir(&repo)
        .status()
        .unwrap();
    Command::new("git")
        .args([
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@localhost",
            "commit",
            "-qm",
            "baseline",
        ])
        .current_dir(&repo)
        .status()
        .unwrap();
    let original_ref = Command::new("git")
        .args(["rev-parse", "refs/heads/main"])
        .current_dir(&repo)
        .output()
        .unwrap()
        .stdout;

    let mut worker = WorkerClient::start(&root).unwrap();
    let before = worker
        .request(
            RequestKind::ResolveSymbol,
            &json!({"repo":repo,"symbol":"com.acme.namedCall"}),
        )
        .unwrap();
    let symbol_id = before["declaration"]["symbolId"].clone();
    let body_anchor = before["bodyAnchor"]["anchorId"].clone();

    let path = repo.join("src/main/kotlin/com/acme/Samples.kt");
    let source = std::fs::read_to_string(&path).unwrap();
    let changed = source
        .replace(
            "fun classify",
            "// inserted neighbor comment\n\nfun classify",
        )
        .replace("fun namedCall(value", "fun namedCall( value");
    std::fs::write(&path, changed).unwrap();
    let after = worker
        .request(
            RequestKind::ResolveSymbol,
            &json!({"repo":repo,"symbol":"com.acme.namedCall"}),
        )
        .unwrap();
    assert_eq!(after["declaration"]["symbolId"], symbol_id);
    assert_eq!(after["bodyAnchor"]["anchorId"], body_anchor);

    let current = std::fs::read_to_string(&path).unwrap();
    let before_offset = current.find("return value").unwrap() + 2;
    let return_before = worker.request(RequestKind::ResolveExpression, &json!({"repo":repo,"file":"src/main/kotlin/com/acme/Samples.kt","offset":before_offset})).unwrap();
    let return_anchor = return_before["anchor"]["anchorId"].clone();
    std::fs::write(
        &path,
        current.replace("    if (premium)", "    val neighbor = 1\n    if (premium)"),
    )
    .unwrap();
    let changed = std::fs::read_to_string(&path).unwrap();
    let after_offset = changed.find("return value").unwrap() + 2;
    let return_after = worker.request(RequestKind::ResolveExpression, &json!({"repo":repo,"file":"src/main/kotlin/com/acme/Samples.kt","offset":after_offset})).unwrap();
    assert_eq!(return_after["anchor"]["anchorId"], return_anchor);

    let project = worker
        .request(RequestKind::OpenProject, &json!({"repo":repo}))
        .unwrap();
    let raw = worker
        .request(
            RequestKind::BuildLocalGraph,
            &json!({"repo":repo,"symbol":"com.acme.total"}),
        )
        .unwrap();
    let graph = graph::enrich(serde_json::from_value::<LocalGraph>(raw).unwrap());
    let seed = graph
        .nodes
        .iter()
        .find(|node| node.kind == "RETURN")
        .unwrap()
        .id
        .clone();
    let thread = graph::slice(
        &graph,
        &seed,
        SlicePolicy::default(),
        Snapshot {
            base_revision: "test".into(),
            project_model_hash: project["projectModelHash"].as_str().unwrap().into(),
            compiler_version: "2.4.10".into(),
        },
        json!({"kind":"FUNCTION_RETURN","symbol":"com.acme.total","nodeId":seed}),
    )
    .unwrap();
    let first = canonical::bytes(&thread).unwrap();
    let round_trip: ThreadIr = serde_json::from_slice(&first).unwrap();
    assert_eq!(first, canonical::bytes(&round_trip).unwrap());

    let target = after["bodyAnchor"].clone();
    let source_text = target["sourceText"].as_str().unwrap();
    let request = json!({
        "repo":repo,"file":target["fileId"],"ownerSymbolId":target["ownerSymbolId"],
        "exactTextHash":target["exactTextHash"],"syntaxKind":target["syntaxKind"],
        "normalizedTokenHash":target["normalizedTokenHash"],"kind":"REPLACE_EXPRESSION",
        "replacement":source_text,"preconditions":{},"postconditions":{"typeAssignableTo":"String"}
    });
    let no_op = worker.request(RequestKind::ApplyEdit, &request).unwrap();
    assert_eq!(no_op["originalHash"], no_op["candidateHash"]);
    assert_eq!(
        no_op["source"].as_str().unwrap().as_bytes(),
        std::fs::read(&path).unwrap()
    );

    let mut wrong_type = request.clone();
    wrong_type["postconditions"] = json!({"typeAssignableTo":"Int"});
    let error = worker
        .request(RequestKind::ApplyEdit, &wrong_type)
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::TypeMismatch);
    let mut invalid = request;
    invalid["replacement"] = json!("value =");
    let error = worker
        .request(RequestKind::ApplyEdit, &invalid)
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::ReplacementParseError);
    let import_request = json!({
        "repo":repo,"file":"src/main/kotlin/com/acme/Samples.kt","kind":"ADD_IMPORT",
        "replacement":"java.time.Instant","source":std::fs::read_to_string(&path).unwrap()
    });
    let imported_once = worker
        .request(RequestKind::ApplyEdit, &import_request)
        .unwrap();
    let mut imported_twice_request = import_request;
    imported_twice_request["source"] = imported_once["source"].clone();
    let imported_twice = worker
        .request(RequestKind::ApplyEdit, &imported_twice_request)
        .unwrap();
    assert_eq!(imported_once["source"], imported_twice["source"]);
    let final_ref = Command::new("git")
        .args(["rev-parse", "refs/heads/main"])
        .current_dir(&repo)
        .output()
        .unwrap()
        .stdout;
    assert_eq!(original_ref, final_ref);
    worker.shutdown().unwrap();
}
