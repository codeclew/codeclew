//! Public CLI metadata collection with authenticated descriptors and no tools.
#![cfg(unix)]
#[path = "support/documentation.rs"]
mod support;
use clew::{
    canonical,
    documentation::{check, store::Repository},
};
use serde_json::{Value, json};
use std::{collections::BTreeMap, fs, os::unix::fs::PermissionsExt, path::Path};
use support::Fixture;

#[test]
fn private_cache_bundle_replays_saved_work_offline_without_git_or_source_bindings() {
    let f = Fixture::new();
    let source = f.service("orders");
    let checked = f.checked();
    let repo = Repository::open(&f.docs).unwrap();
    let snapshot = checked.save_snapshot(&repo).unwrap();
    drop(repo);
    let request=f.input("support-work.json", &json!({"schema":"codeclew-documentation-work-request/1.0","audience":"Service maintainer","maxItems":100,"maxBytes":49152}));
    let prepared = f.ok(&[
        "docs",
        "work",
        "prepare",
        "--subject",
        "service:orders",
        "--snapshot",
        &snapshot,
        "--input",
        request.to_str().unwrap(),
    ]);
    let work = prepared["work"].as_str().unwrap();
    // Saved diagnostics do not need a new provider invocation. Derive copied
    // paths from bounded IDs; never follow a persisted outputDirectory.
    let run = "a".repeat(32);
    let invocation = "b".repeat(32);
    let proposal = "c".repeat(64);
    let saved = [
        (
            format!(".codeclew/work/{work}/latest-run.json"),
            json!({"run":run}),
        ),
        (
            format!(".codeclew/jobs/{run}.json"),
            json!({"run":run,"work":work,"proposal":proposal,"attempts":[{"invocation":invocation}],"draft":{"outputDirectory":source.join(".git")}}),
        ),
        (
            format!(".codeclew/jobs/{run}/checkpoints/saved.json"),
            json!({"checkpoint":"saved failure"}),
        ),
        (
            format!(".codeclew/job-inputs/{invocation}.json"),
            json!({"request":"frozen input"}),
        ),
        (
            format!(".codeclew/job-results/{invocation}.json"),
            json!({"result":"frozen output"}),
        ),
        (
            format!(".codeclew/job-model-inputs/{invocation}.json"),
            json!({"modelInput":"frozen wire input"}),
        ),
        (
            format!(".codeclew/job-model-wire-results/{invocation}.json"),
            json!({"result":"frozen wire output"}),
        ),
        (
            format!(".codeclew/drafts/{work}/answer.json"),
            json!({"answer":"saved draft"}),
        ),
        (
            format!(".codeclew/model-id-maps/{run}/author/head.json"),
            json!({"map":"frozen IDs"}),
        ),
        (
            format!(".codeclew/proposals/{proposal}.json"),
            json!({"proposal":"saved proposal"}),
        ),
    ];
    for (name, value) in &saved {
        let path = f.docs.join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, serde_json::to_vec(value).unwrap()).unwrap();
    }
    fs::write(
        source.join(".git/private-sentinel"),
        "PRIVATE_GIT_MUST_NOT_BE_COPIED",
    )
    .unwrap();
    fs::rename(&source, source.with_extension("offline")).unwrap();
    let before = tree(&f.docs);
    let output = f.temp.path().join("cache-bundle");
    let summary = f.ok(&[
        "support",
        "collect",
        "--work",
        work,
        "--include-cache",
        "--output",
        output.to_str().unwrap(),
    ]);
    assert_eq!(summary["sharing"], "PRIVATE_SOURCE_CACHE");
    assert_eq!(summary["snapshot"], snapshot);
    assert_eq!(before, tree(&f.docs));
    for (name, _) in &saved {
        assert_eq!(
            fs::read(output.join("PRIVATE_CACHE").join(name)).unwrap(),
            fs::read(f.docs.join(name)).unwrap()
        );
    }
    let manifest: Value =
        serde_json::from_slice(&fs::read(output.join("manifest.json")).unwrap()).unwrap();
    for artifact in manifest["artifacts"].as_array().unwrap() {
        let name = artifact["path"].as_str().unwrap();
        assert!(
            !name.contains(".git")
                && !name.contains("/bindings/")
                && !name.contains("private-sentinel")
        );
        let bytes = fs::read(output.join(name)).unwrap();
        assert_eq!(artifact["digest"], canonical::hash_bytes(&bytes));
        assert_eq!(artifact["bytes"], bytes.len());
        assert_eq!(
            fs::metadata(output.join(name))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
    let selection = f.input(
        "support-selection.json",
        &json!({"query":{"kind":"SYMBOL","symbolContains":"reserve"}}),
    );
    let (code, page) = f.run_unrooted(&[
        "docs",
        "work",
        "read",
        "--root",
        output.join("PRIVATE_CACHE").to_str().unwrap(),
        "--work",
        work,
        "--input",
        selection.to_str().unwrap(),
    ]);
    assert_eq!(code, 0, "{page}");
    assert_eq!(page["snapshot"], snapshot);
    assert!(!page["items"].as_array().unwrap().is_empty());
    let restored = Repository::open(&output.join("PRIVATE_CACHE")).unwrap();
    let actual = check::Check::load_snapshot(&restored, &snapshot).unwrap();
    assert_eq!(
        actual.services["orders"].sources,
        checked.services["orders"].sources
    );
    assert_eq!(before, tree(&f.docs));
}

#[test]
fn cache_budget_and_symlink_refusals_leave_source_and_metadata_diagnostics_available() {
    let f = Fixture::new();
    f.service("orders");
    f.checked();
    let before = tree(&f.docs);
    let output = f.temp.path().join("small-cache-bundle");
    let summary = f.ok(&[
        "support",
        "collect",
        "--include-cache",
        "--cache-max-bytes",
        "1",
        "--output",
        output.to_str().unwrap(),
    ]);
    assert_eq!(summary["status"], "PARTIAL");
    assert_eq!(before, tree(&f.docs));
    let report: Value =
        serde_json::from_slice(&fs::read(output.join("report.json")).unwrap()).unwrap();
    assert_eq!(report["cache"]["status"], "NOT_INCLUDED");
    assert!(!output.join("PRIVATE_CACHE").exists());
    let secret = f.temp.path().join("not-cache.json");
    fs::write(&secret, "PRIVATE_NON_CACHE_SENTINEL").unwrap();
    std::os::unix::fs::symlink(&secret, f.docs.join(".codeclew/cache/escape.json")).unwrap();
    let output = f.temp.path().join("symlink-cache-bundle");
    let summary = f.ok(&[
        "support",
        "collect",
        "--include-cache",
        "--output",
        output.to_str().unwrap(),
    ]);
    assert_eq!(summary["status"], "PARTIAL");
    assert!(!output.join("PRIVATE_CACHE").exists());
    assert!(output.join("report.json").exists());
    assert_eq!(fs::read(&secret).unwrap(), b"PRIVATE_NON_CACHE_SENTINEL");
}

#[test]
fn stable_committed_wal_is_copied_without_checkpointing_the_source() {
    let f = Fixture::new();
    f.service("orders");
    f.checked();
    let layout: Value = serde_json::from_slice(
        &fs::read(f.docs.join(".codeclew/cache/object-layout.json")).unwrap(),
    )
    .unwrap();
    let database = layout["database"].as_str().unwrap();
    let connection = rusqlite::Connection::open(f.docs.join(database)).unwrap();
    connection
        .execute_batch("PRAGMA wal_autocheckpoint=0")
        .unwrap();
    let bytes = b"Committed WAL diagnostic fixture";
    let digest = canonical::hash_bytes(bytes);
    connection
        .execute(
            "INSERT INTO objects (payload, byte_len, digest) VALUES (?1, ?2, ?3)",
            rusqlite::params![bytes.as_slice(), bytes.len() as i64, digest],
        )
        .unwrap();
    let wal = format!("{database}-wal");
    assert!(fs::metadata(f.docs.join(&wal)).unwrap().len() > 0);
    let before = tree(&f.docs);
    let output = f.temp.path().join("wal-cache-bundle");
    let summary = f.ok(&[
        "support",
        "collect",
        "--include-cache",
        "--output",
        output.to_str().unwrap(),
    ]);
    assert_eq!(summary["status"], "PARTIAL");
    assert_eq!(before, tree(&f.docs));
    assert_eq!(
        fs::read(output.join("PRIVATE_CACHE").join(&wal)).unwrap(),
        before[&f.docs.join(&wal)]
    );
    assert!(
        !output
            .join("PRIVATE_CACHE")
            .join(format!("{database}-shm"))
            .exists()
    );
    let copied = rusqlite::Connection::open(output.join("PRIVATE_CACHE").join(database)).unwrap();
    let actual: Vec<u8> = copied
        .query_row(
            "SELECT payload FROM objects WHERE digest=?1",
            [&digest],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(actual, bytes);
    assert_eq!(before, tree(&f.docs));
    drop(connection);
}

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
