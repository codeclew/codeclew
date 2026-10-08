//! Public CLI metadata collection with authenticated descriptors and no tools.
#![cfg(unix)]
#[path = "support/documentation.rs"]
mod support;
use clew::documentation::{check, store::Repository};
use serde_json::Value;
use std::{collections::BTreeMap, fs, os::unix::fs::PermissionsExt, path::Path};
use support::Fixture;

fn tree(root: &Path) -> BTreeMap<std::path::PathBuf, Vec<u8>> {
    let mut files = BTreeMap::new();
    for entry in fs::read_dir(root).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            files.extend(tree(&path));
        } else {
            files.insert(path.clone(), fs::read(path).unwrap());
        }
    }
    files
}

#[test]
fn collector_uses_exact_historical_manifest_with_no_tools_and_unchanged_store() {
    let f = Fixture::new();
    let source = f.service("orders");
    let checked = f.checked();
    let repo = Repository::open(&f.docs).unwrap();
    let snapshot = checked.save_snapshot(&repo).unwrap();
    drop(repo);
    let manifest: Value = serde_json::from_slice(
        &fs::read(f.docs.join(".codeclew/cache/latest-check.json")).unwrap(),
    )
    .unwrap();
    let layout: Value = serde_json::from_slice(
        &fs::read(f.docs.join(".codeclew/cache/object-layout.json")).unwrap(),
    )
    .unwrap();
    let connection =
        rusqlite::Connection::open(f.docs.join(layout["database"].as_str().unwrap())).unwrap();
    connection
        .execute(
            "DELETE FROM objects WHERE digest = ?1",
            [manifest["dependenciesIndex"]["digest"].as_str().unwrap()],
        )
        .unwrap();
    drop(connection); // Missing heavy object is intentionally NOT_VERIFIED.
    fs::rename(&source, source.with_extension("offline")).unwrap();
    fs::write(
        f.docs.join("codeclew-docs.yaml"),
        "current: invalid declaration",
    )
    .unwrap();
    fs::write(
        f.docs.join(".codeclew/cache/latest-check.json"),
        "new broken latest",
    )
    .unwrap();
    let tools = f.temp.path().join("no-tools");
    fs::create_dir(&tools).unwrap();
    let invoked = f.temp.path().join("invoked");
    for name in [
        "git", "java", "dotnet", "cargo", "rustc", "mvn", "node", "curl",
    ] {
        let script = tools.join(name);
        fs::write(
            &script,
            format!(
                "#!/bin/sh\necho invoked >> '{}'\nexit 91\n",
                invoked.display()
            ),
        )
        .unwrap();
        fs::set_permissions(script, fs::Permissions::from_mode(0o700)).unwrap();
    }
    let before = tree(&f.docs);
    let output = f.temp.path().join("bundle");
    let result = f.run_raw_with_path(
        &[
            "support",
            "collect",
            "--snapshot",
            &snapshot,
            "--output",
            output.to_str().unwrap(),
        ],
        &tools,
    );
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stdout)
    );
    let summary: Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(summary["status"], "COMPLETE");
    assert_eq!(summary["snapshot"], snapshot);
    assert_eq!(before, tree(&f.docs));
    assert!(!invoked.exists());
    let report: Value =
        serde_json::from_slice(&fs::read(output.join("report.json")).unwrap()).unwrap();
    assert_eq!(report["checkStatus"], "CHECKED");
    assert_eq!(report["runtime"]["mode"], "DEVELOPMENT");
    assert_eq!(report["currentDeclarations"], "NOT_INSPECTED");
    assert_eq!(report["heavyObjects"], "NOT_VERIFIED");
    assert!(report["services"][0]["entrypoints"].as_u64().unwrap() > 0);
    assert!(!report.to_string().contains("class Worker"));
    let repeat = f.run(&[
        "support",
        "collect",
        "--snapshot",
        &snapshot,
        "--output",
        output.to_str().unwrap(),
    ]);
    assert_eq!(repeat.0, 2);
    assert_eq!(before, tree(&f.docs));
}

#[test]
fn unresolved_check_collects_original_typed_reason_and_partial_missing_metadata() {
    let f = Fixture::new();
    f.service("unbound");
    fs::remove_file(f.docs.join(".codeclew/bindings/unbound.json")).unwrap();
    // This is a real source-session admission failure, not a compiler success.
    let repo = Repository::open(&f.docs).unwrap();
    let checked = check::run(&repo).unwrap();
    assert_eq!(checked.unresolved["unbound"]["reason"], "INVALID_INPUT");
    let snapshot = checked.save_snapshot(&repo).unwrap();
    drop(repo);
    let before = tree(&f.docs);
    let output = f.temp.path().join("failed");
    let (code, summary) = f.run(&["support", "collect", "--output", output.to_str().unwrap()]);
    assert_eq!(code, 0);
    assert_eq!(summary["status"], "COMPLETE");
    assert_eq!(summary["snapshot"], snapshot);
    let report: Value =
        serde_json::from_slice(&fs::read(output.join("report.json")).unwrap()).unwrap();
    assert_eq!(report["checkStatus"], "UNRESOLVED");
    assert_eq!(report["services"][0]["failure"]["reason"], "INVALID_INPUT");
    assert_eq!(report["services"][0]["selected"], true);
    assert_eq!(before, tree(&f.docs));
    // Selected minimal metadata is missing; don't scan unrelated fact objects.
    let layout: Value = serde_json::from_slice(
        &fs::read(f.docs.join(".codeclew/cache/object-layout.json")).unwrap(),
    )
    .unwrap();
    let connection =
        rusqlite::Connection::open(f.docs.join(layout["database"].as_str().unwrap())).unwrap();
    let manifest: Value = serde_json::from_slice(
        &fs::read(f.docs.join(".codeclew/cache/latest-check.json")).unwrap(),
    )
    .unwrap();
    connection
        .execute(
            "DELETE FROM objects WHERE digest = ?1",
            [manifest["sourceInputs"]["digest"].as_str().unwrap()],
        )
        .unwrap();
    drop(connection);
    let before_missing = tree(&f.docs);
    let missing = f.temp.path().join("missing");
    let (code, summary) = f.run(&[
        "support",
        "collect",
        "--snapshot",
        &snapshot,
        "--output",
        missing.to_str().unwrap(),
    ]);
    assert_eq!(code, 0);
    assert_eq!(summary["status"], "PARTIAL");
    assert_eq!(summary["snapshot"], snapshot);
    assert_eq!(before_missing, tree(&f.docs));
    let connection =
        rusqlite::Connection::open(f.docs.join(layout["database"].as_str().unwrap())).unwrap();
    connection
        .execute(
            "UPDATE objects SET payload = ?1 WHERE digest = ?2",
            rusqlite::params![b"{}".as_slice(), snapshot.rsplit_once('/').unwrap().0],
        )
        .unwrap();
    drop(connection);
    let before_corrupt = tree(&f.docs);
    let corrupt = f.temp.path().join("corrupt");
    let (code, summary) = f.run(&[
        "support",
        "collect",
        "--snapshot",
        &snapshot,
        "--output",
        corrupt.to_str().unwrap(),
    ]);
    assert_eq!(code, 0);
    assert_eq!(summary["status"], "PARTIAL");
    assert_eq!(before_corrupt, tree(&f.docs));
}

#[test]
fn invocation_hook_uses_new_failed_check_and_retained_reader_error_snapshot() {
    let f = Fixture::new();
    f.service("orders");
    let baseline = f.checked();
    let repo = Repository::open(&f.docs).unwrap();
    let old_snapshot = baseline.save_snapshot(&repo).unwrap();
    drop(repo);
    fs::remove_file(f.docs.join(".codeclew/bindings/orders.json")).unwrap();
    let output = f.temp.path().join("hook");
    fs::create_dir(&output).unwrap();
    fs::set_permissions(&output, fs::Permissions::from_mode(0o700)).unwrap();
    let result = f.run_raw_with_diagnostics(&["docs", "check", "--service", "orders"], &output);
    assert_eq!(result.status.code(), Some(3));
    let value: Value = serde_json::from_slice(&result.stdout).unwrap();
    let snapshot = value["snapshot"].as_str().unwrap();
    assert_ne!(snapshot, old_snapshot);
    let report: Value =
        serde_json::from_slice(&fs::read(output.join("core-report.json")).unwrap()).unwrap();
    assert_eq!(report["snapshot"], snapshot);
    assert_eq!(report["checkStatus"], "UNRESOLVED");
    assert_eq!(report["services"][0]["failure"]["reason"], "INVALID_INPUT");
    let error_output = f.temp.path().join("reader-hook");
    fs::create_dir(&error_output).unwrap();
    fs::set_permissions(&error_output, fs::Permissions::from_mode(0o700)).unwrap();
    let error = f.run_raw_with_diagnostics(
        &[
            "docs",
            "context",
            "--service",
            "orders",
            "--snapshot",
            snapshot,
        ],
        &error_output,
    );
    assert_eq!(error.status.code(), Some(2));
    let value: Value = serde_json::from_slice(&error.stdout).unwrap();
    assert_eq!(value["error"]["snapshotId"], snapshot);
    let report: Value =
        serde_json::from_slice(&fs::read(error_output.join("core-report.json")).unwrap()).unwrap();
    assert_eq!(report["snapshot"], snapshot);
    assert_eq!(report["snapshotSource"], "ERROR_CONTEXT_NOT_RECOLLECTED");
    assert_eq!(report["recordedFailure"]["reason"], "INVALID_INPUT");
}
