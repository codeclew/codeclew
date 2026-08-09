use serde_json::{Value, json};
use std::path::Path;
use sthread::canonical;
use sthread::identity::{IdentityLifecycle, SnapshotProvenance, decide_identity_delta};
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

fn provenance(index: &Value) -> SnapshotProvenance {
    let project_model = index["projectModelHash"].as_str().unwrap().to_owned();
    let classpath = index["classpathHash"].as_str().unwrap().to_owned();
    let options = index["compilerOptionsHash"].as_str().unwrap().to_owned();
    let index_hash = index["indexHash"].as_str().unwrap().to_owned();
    SnapshotProvenance {
        composite_snapshot_hash: canonical::hash(&json!({
            "index":index_hash,
            "projectModel":project_model,
            "classpath":classpath,
            "compilerOptions":options,
        }))
        .unwrap(),
        index_snapshot_hash: index_hash,
        project_model_hash: project_model,
        classpath_hash: classpath,
        compiler_options_hash: options,
    }
}

#[test]
fn kotlin_21_worker_rename_is_unique_and_source_provenanced() {
    let root = workspace_root();
    let temp = tempfile::tempdir().unwrap();
    let repo = temp.path().join("kotlin-2-1");
    copy_fixture(
        &root.join("fixtures/kotlin-basic"),
        &temp.path().join("kotlin-basic"),
    );
    copy_fixture(&root.join("fixtures/kotlin-2-1"), &repo);
    let mut worker = WorkerClient::start(&root).unwrap();
    let before = worker
        .request(
            RequestKind::IndexFiles,
            &json!({"repo":repo,"compilation":":/main"}),
        )
        .unwrap();

    let source_path = repo.join("src/main/kotlin/com/acme/Options.kt");
    let source = std::fs::read_to_string(&source_path).unwrap();
    std::fs::write(&source_path, source.replace("readOptions", "loadOptions")).unwrap();
    let after = worker
        .request(
            RequestKind::IndexFiles,
            &json!({"repo":repo,"compilation":":/main"}),
        )
        .unwrap();

    let report = decide_identity_delta(
        provenance(&before),
        provenance(&after),
        before["files"].as_array().unwrap(),
        after["files"].as_array().unwrap(),
    )
    .unwrap();
    let rename = report
        .decisions
        .iter()
        .find(|decision| {
            decision
                .before
                .first()
                .is_some_and(|identity| identity.name == "readOptions")
        })
        .unwrap();
    assert_eq!(rename.lifecycle, IdentityLifecycle::Renamed);
    assert_eq!(rename.after[0].name, "loadOptions");
    assert!(!rename.before[0].source.content_hash.is_empty());
    assert!(!rename.after[0].source.content_hash.is_empty());
    assert_ne!(
        rename.before[0].source.content_hash,
        rename.after[0].source.content_hash
    );
    assert!(
        report
            .introduced
            .iter()
            .all(|item| item.name != "loadOptions")
    );
    worker.shutdown().unwrap();
}
