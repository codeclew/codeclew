//! Retained consumers run in fresh CLI processes with the source unavailable.
#![cfg(unix)]
#[path = "support/documentation.rs"]
mod support;
use clew::documentation::{cache, check::Check, proposals, store::Repository, work};
use serde_json::{Value, json};
use std::fs;
use support::{Fixture, read};

fn capture(f: &Fixture, service: &str) -> String {
    let (_, value) = f.run(&["docs", "check", "--service", service]);
    value["snapshot"]
        .as_str()
        .unwrap_or_else(|| panic!("{value}"))
        .to_owned()
}
fn prepare(f: &Fixture, snapshot: &str, entrypoint: Option<&str>) -> Value {
    let request = f.input(
        "request.json",
        &json!({
            "schema":"codeclew-documentation-work-request/1.0", "audience":"Maintainers",
            "entrypoint":entrypoint, "maxItems":100, "maxBytes":49152
        }),
    );
    f.ok(&[
        "docs",
        "work",
        "prepare",
        "--subject",
        "service:orders",
        "--input",
        request.to_str().unwrap(),
        "--snapshot",
        snapshot,
    ])
}
fn read_all(f: &Fixture, mut page: Value) -> String {
    let id = page["work"].as_str().unwrap().to_owned();
    while let Some(cursor) = page["nextCursor"].as_str() {
        let input = f.input("read.json", &json!({"cursor":cursor}));
        page = f.ok(&[
            "docs",
            "work",
            "read",
            "--work",
            &id,
            "--input",
            input.to_str().unwrap(),
        ]);
    }
    id
}

fn object_database(repo: &Repository) -> std::path::PathBuf {
    let marker: Value = serde_json::from_slice(
        &fs::read(repo.root.join(".codeclew/cache/object-layout.json")).unwrap(),
    )
    .unwrap();
    repo.root.join(marker["database"].as_str().unwrap())
}

#[test]
fn retained_work_reads_survive_latest_replacement_and_unavailable_sources() {
    let f = Fixture::new();
    let orders = f.service("orders");
    let other = f.service("other");
    let a = capture(&f, "orders");
    let repo = Repository::open(&f.docs).unwrap();
    let checked = Check::load_snapshot(&repo, &a).unwrap();
    let entry = checked.services["orders"]
        .entrypoints
        .iter()
        .find(|e| e.symbol.contains("reserve"))
        .unwrap();
    let b = capture(&f, "other");
    assert_ne!(a, b);
    let latest = f.docs.join(".codeclew/cache/latest-check.json");
    let latest_b = fs::read(&latest).unwrap();
    // Successful reads must retain A even when acquisition cannot access either repository.
    fs::rename(&orders, orders.with_extension("offline")).unwrap();
    fs::rename(&other, other.with_extension("offline")).unwrap();
    let context = f.ok(&["docs", "context", "--service", "orders", "--snapshot", &a]);
    assert_eq!(context["contextDigest"], checked.context_digest);
    assert_eq!(context["authority"], "PINNED_SNAPSHOT_NOT_REVERIFIED");
    let page = prepare(&f, &a, Some(&entry.id));
    assert_eq!(page["snapshot"], a);
    let id = read_all(&f, page);
    let frozen = work::load(&repo, &id).unwrap();
    assert_eq!(frozen.checked.services.len(), 1);
    assert_eq!(
        frozen.checked.services["orders"].revision,
        checked.services["orders"].revision
    );
    proposals::current(&repo, &frozen).unwrap();
    assert_eq!(fs::read(&latest).unwrap(), latest_b);
    assert_eq!(
        Check::load_snapshot(&repo, &a).unwrap().context_digest,
        checked.context_digest
    );
}

#[test]
fn retained_snapshot_corruption_and_missing_payload_never_fall_back_to_latest() {
    let f = Fixture::new();
    f.service("orders");
    let snapshot = capture(&f, "orders");
    let repo = Repository::open(&f.docs).unwrap();
    let checked = Check::load_snapshot(&repo, &snapshot).unwrap();
    let latest = f.docs.join(".codeclew/cache/latest-check.json");
    let latest_bytes = fs::read(&latest).unwrap();
    let manifest = checked.store_manifest(&repo).unwrap();
    let digest = &manifest.service_manifests["orders"].sources.digest;
    let original = cache::get(
        &repo,
        &manifest.service_manifests["orders"].sources,
        128 * 1024 * 1024,
    )
    .unwrap()
    .unwrap();
    let mut corrupt = original.clone();
    corrupt[0] ^= 1;
    rusqlite::Connection::open(object_database(&repo))
        .unwrap()
        .execute(
            "UPDATE objects SET payload = ?1 WHERE digest = ?2",
            rusqlite::params![corrupt.as_slice(), digest],
        )
        .unwrap();
    assert!(Check::load_snapshot(&repo, &snapshot).is_err());
    assert_ne!(
        f.run(&[
            "docs",
            "context",
            "--service",
            "orders",
            "--snapshot",
            &snapshot
        ])
        .0,
        0
    );
    rusqlite::Connection::open(object_database(&repo))
        .unwrap()
        .execute(
            "DELETE FROM objects WHERE digest = ?1",
            rusqlite::params![digest],
        )
        .unwrap();
    assert!(Check::load_snapshot(&repo, &snapshot).is_err());
    assert!(!f.docs.join("docs/index.html").exists());
    assert_eq!(fs::read(latest).unwrap(), latest_bytes);
    for invalid in [
        "../latest-check.json",
        "sha256:bad/10",
        "sha256:bad/+10",
        "sha256:bad/010",
    ] {
        assert!(Check::load_snapshot(&repo, invalid).is_err());
    }
}

#[test]
fn retained_work_rejects_changed_declarations_and_unrelated_service_capture() {
    let f = Fixture::new();
    f.service("orders");
    f.service("other");
    let checked = f.checked();
    let repo = Repository::open(&f.docs).unwrap();
    let all = checked.save_snapshot(&repo).unwrap();
    let request: work::Request = serde_json::from_value(
        json!({"schema":"codeclew-documentation-work-request/1.0", "audience":"Maintainers"}),
    )
    .unwrap();
    let prepared =
        work::prepare_with_snapshot(&repo, "service:orders".into(), request.clone(), Some(&all))
            .unwrap();
    let full = work::load(&repo, prepared["work"].as_str().unwrap()).unwrap();
    assert_eq!(full.snapshot.as_deref(), Some(all.as_str()));
    assert_eq!(
        serde_json::to_value(&full.checked).unwrap(),
        serde_json::to_value(&checked).unwrap()
    );
    let a = capture(&f, "orders");
    let id = read_all(&f, prepare(&f, &a, None));
    let frozen = work::load(&repo, &id).unwrap();
    fs::create_dir_all(f.docs.join("notes")).unwrap();
    fs::write(f.docs.join("notes/context.md"), "Changed author input").unwrap();
    assert!(proposals::current(&repo, &frozen).is_err());
    // Human files are separate authoring inputs; a new work may capture their
    // changed text. A changed service declaration invalidates the snapshot.
    let declaration_path = f.docs.join("catalog/services/orders.json");
    let mut declaration = read(&declaration_path);
    declaration["source"]["roots"] = json!(["src"]);
    fs::write(&declaration_path, serde_json::to_vec(&declaration).unwrap()).unwrap();
    assert!(
        work::prepare_with_snapshot(&repo, "service:orders".into(), request, Some(&a)).is_err()
    );
    assert_ne!(
        f.run(&["docs", "context", "--service", "orders", "--snapshot", &a])
            .0,
        0
    );
}

#[test]
fn unsupported_draft_request_never_reacquires_evidence_or_publishes() {
    let f = Fixture::new();
    let source = f.service("orders");
    let snapshot = capture(&f, "orders");
    let page = prepare(&f, &snapshot, None);
    let id = page["work"].as_str().unwrap();
    fs::rename(&source, source.with_extension("offline")).unwrap();
    let latest = f.docs.join(".codeclew/cache/latest-check.json");
    fs::write(&latest, b"generation gap must use retained snapshot").unwrap();
    let (code, result) = f.run(&["docs", "work", "run", "--draft", "--work", id]);
    assert_ne!(code, 0);
    assert!(
        result
            .to_string()
            .contains("OPERATION_DRAFT_PREPARE_REQUIRED"),
        "{result}"
    );
    assert!(!f.docs.join("docs/index.html").exists());
    assert_eq!(
        fs::read(latest).unwrap(),
        b"generation gap must use retained snapshot"
    );
    let repo = Repository::open(&f.docs).unwrap();
    assert_eq!(
        Check::load_snapshot(&repo, &snapshot)
            .unwrap()
            .services
            .len(),
        1
    );
}

#[test]
fn portable_retained_export_cli_roundtrips_without_the_source_checkout() {
    let producer = Fixture::new();
    let checkout = producer.service("orders");
    let snapshot = capture(&producer, "orders");
    let repo = Repository::open(&producer.docs).unwrap();
    let original = Check::load_snapshot(&repo, &snapshot).unwrap();
    let expected = original.services["orders"].clone();
    let service: Value = read(producer.docs.join("catalog/services/orders.json"));
    fs::rename(&checkout, checkout.with_extension("offline")).unwrap();
    let package = producer.temp.path().join("retained-package");
    let exported = producer.ok(&[
        "docs",
        "evidence",
        "export",
        "--service",
        "orders",
        "--snapshot",
        &snapshot,
        "--output",
        package.to_str().unwrap(),
    ]);
    assert_eq!(exported["status"], "EXPORTED_RETAINED_SNAPSHOT");
    assert_eq!(exported["retained"]["snapshot"], snapshot);
    assert_eq!(
        exported["retained"]["sourceAuthority"],
        "RETAINED_SOURCE_NOT_REVERIFIED"
    );
    let (code, inspected) = producer.run_unrooted(&[
        "docs",
        "evidence",
        "inspect",
        "--input",
        package.to_str().unwrap(),
    ]);
    assert_eq!(code, 0, "{inspected}");
    assert_eq!(inspected["status"], "INTEGRITY_CHECKED_NOT_ADMITTED");
    let (code, page) = producer.run_unrooted(&[
        "docs",
        "evidence",
        "read",
        "--input",
        package.to_str().unwrap(),
        "--kind",
        "sources",
        "--limit",
        "20",
    ]);
    assert_eq!(code, 0, "{page}");
    assert!(!page["items"].as_array().unwrap().is_empty());

    let consumer = Fixture::new();
    let input = consumer.input("original-service.json", &service);
    let digest = consumer.ok(&["docs", "service", "list"])["inputDigest"]
        .as_str()
        .unwrap()
        .to_owned();
    consumer.ok(&[
        "docs",
        "service",
        "add",
        "--input",
        input.to_str().unwrap(),
        "--expected-input-digest",
        &digest,
    ]);
    assert_ne!(
        consumer
            .run(&[
                "docs",
                "evidence",
                "import",
                "--input",
                package.to_str().unwrap()
            ])
            .0,
        0
    );
    let expectation = consumer.input("trusted-expectation.json", &json!({
        "schema":"codeclew-documentation-evidence-expectation/1.0", "service":"orders",
        "repositoryId":service["repositoryId"], "serviceDigest":exported["serviceDigest"],
        "revision":exported["revision"], "manifestDigest":exported["manifestDigest"], "sequence":1,
    }));
    let digest = consumer.ok(&["docs", "service", "list"])["inputDigest"]
        .as_str()
        .unwrap()
        .to_owned();
    consumer.ok(&[
        "docs",
        "evidence",
        "expect",
        "--input",
        expectation.to_str().unwrap(),
        "--expected-input-digest",
        &digest,
    ]);
    let imported = consumer.ok(&[
        "docs",
        "evidence",
        "import",
        "--input",
        package.to_str().unwrap(),
    ]);
    assert_eq!(imported["status"], "IMPORTED_RETAINED_SNAPSHOT");
    let (code, checked) = consumer.run(&["docs", "check", "--service", "orders"]);
    assert!(matches!(code, 0 | 3 | 4), "{checked}");
    let repo = Repository::open(&consumer.docs).unwrap();
    let result = Check::load_snapshot(&repo, checked["snapshot"].as_str().unwrap()).unwrap();
    assert_eq!(
        result.source_authorities()["orders"],
        "RETAINED_SOURCE_NOT_REVERIFIED"
    );
    let actual = &result.services["orders"];
    assert_eq!(actual.sources, expected.sources);
    assert_eq!(actual.entrypoints, expected.entrypoints);
    assert_eq!(actual.contracts, expected.contracts);
    for (id, observation) in &expected.observations {
        assert_eq!(actual.observations.get(id), Some(observation));
    }
    assert_eq!(
        result.store_manifest(&repo).unwrap().service_manifests["orders"].cacheability,
        "NON_CACHEABLE"
    );
}

/// Export-only preflight for a caller-owned historical fixture, using the same
/// authenticated CLI harness as the portable tests. Never captures or edits it.
#[test]
#[ignore = "requires an explicit caller-owned retained fixture and new output"]
fn caller_owned_retained_export_preflight() {
    let root = std::env::var("CODECLEW_TEST_RETAINED_EXPORT_ROOT").expect("owned root");
    let oracle = std::env::var("CODECLEW_TEST_RETAINED_EXPORT_ORACLE").expect("owned oracle");
    let output = std::env::var("CODECLEW_TEST_RETAINED_EXPORT_OUTPUT").expect("new output");
    let fixture: Value = read(oracle);
    let f = Fixture::new();
    let (code, result) = f.run_unrooted(&[
        "docs",
        "evidence",
        "export",
        "--root",
        &root,
        "--service",
        fixture["service"].as_str().unwrap(),
        "--snapshot",
        fixture["snapshot"].as_str().unwrap(),
        "--output",
        &output,
    ]);
    fs::write(
        format!("{output}.result.json"),
        serde_json::to_vec_pretty(&result).unwrap(),
    )
    .unwrap();
    assert_eq!(code, 0, "{result}");
    assert_eq!(result["status"], "EXPORTED_RETAINED_SNAPSHOT");
    assert_eq!(result["retained"]["snapshot"], fixture["snapshot"]);
    assert_eq!(
        result["retained"]["sourceAuthority"],
        "RETAINED_SOURCE_NOT_REVERIFIED"
    );
}

/// Explicit, ignored development gate for an owned large retained fixture. Each
/// lifecycle command runs in a fresh authenticated fixture CLI process; time/RSS
/// excludes compilation and the test parent. It never captures source or calls a model.
#[test]
#[ignore = "requires an owned frozen fixture and a new measurement directory"]
fn caller_owned_byte_scale_lifecycle() {
    use std::{path::PathBuf, process::Command};
    let fixture_root =
        PathBuf::from(std::env::var("CODECLEW_TEST_BYTE_SCALE_ROOT").expect("owned fixture root"));
    let output = PathBuf::from(
        std::env::var("CODECLEW_TEST_BYTE_SCALE_OUTPUT").expect("new measurement directory"),
    );
    assert!(!output.exists());
    fs::create_dir_all(&output).unwrap();
    let oracle: Value = read(fixture_root.join("OWNED-FIXTURE.json"));
    let f = Fixture::new();
    let launcher = f.helper_launcher();
    let measured = |label: &str, args: &[&str], expected: i32| -> Value {
        let mut command = Command::new("/usr/bin/time");
        command
            .args([
                if cfg!(target_os = "macos") {
                    "-l"
                } else {
                    "-v"
                },
                "-o",
            ])
            .arg(output.join(format!("{label}.time")))
            .arg(&launcher)
            .args(args);
        let result = command.output().unwrap();
        fs::write(output.join(format!("{label}.stdout.json")), &result.stdout).unwrap();
        fs::write(output.join(format!("{label}.stderr.log")), &result.stderr).unwrap();
        assert_eq!(
            result.status.code(),
            Some(expected),
            "{label}: {} {}",
            String::from_utf8_lossy(&result.stdout),
            String::from_utf8_lossy(&result.stderr)
        );
        serde_json::from_slice(&result.stdout).unwrap()
    };
    let retained = fixture_root.join("docs");
    let package = output.join("package");
    let export = measured(
        "export",
        &[
            "docs",
            "evidence",
            "export",
            "--root",
            retained.to_str().unwrap(),
            "--service",
            "orders",
            "--snapshot",
            oracle["snapshot"].as_str().unwrap(),
            "--output",
            package.to_str().unwrap(),
        ],
        0,
    );
    assert_eq!(export["retained"]["snapshot"], oracle["snapshot"]);
    assert_eq!(
        export["retained"]["cacheability"],
        oracle["captureCacheability"]
    );
    let inspect = measured(
        "inspect",
        &[
            "docs",
            "evidence",
            "inspect",
            "--input",
            package.to_str().unwrap(),
        ],
        0,
    );
    assert_eq!(inspect["manifestDigest"], export["manifestDigest"]);
    assert_eq!(inspect["report"]["sourceCount"], oracle["sources"]);
    let first = measured(
        "read-first",
        &[
            "docs",
            "evidence",
            "read",
            "--input",
            package.to_str().unwrap(),
            "--kind",
            "observations",
            "--limit",
            "2",
        ],
        0,
    );
    assert_eq!(first["items"].as_array().unwrap().len(), 2);
    assert_eq!(first["total"], oracle["observations"]);
    let prefix = first["nextCursor"]
        .as_str()
        .unwrap()
        .split_once(':')
        .unwrap()
        .0;
    for (label, offset) in [("read-middle", 58_141), ("read-final", 116_281)] {
        let cursor = format!("{prefix}:{offset}");
        let page = measured(
            label,
            &[
                "docs",
                "evidence",
                "read",
                "--input",
                package.to_str().unwrap(),
                "--kind",
                "observations",
                "--limit",
                "2",
                "--cursor",
                &cursor,
            ],
            0,
        );
        assert_eq!(page["total"], oracle["observations"]);
        if label == "read-final" {
            assert!(page["nextCursor"].is_null());
        }
    }
    let consumer = output.join("consumer");
    measured(
        "consumer-init",
        &["docs", "init", "--root", consumer.to_str().unwrap()],
        0,
    );
    let repo = Repository::open(&consumer).unwrap();
    repo.atomic(
        "catalog/services/orders.json",
        &clew::canonical::bytes(&oracle["originalService"]).unwrap(),
    )
    .unwrap();
    let expectation = json!({"schema":"codeclew-documentation-evidence-expectation/1.0","service":"orders","repositoryId":"orders","serviceDigest":export["serviceDigest"],"revision":export["revision"],"manifestDigest":export["manifestDigest"],"sequence":1});
    let expectation_path = output.join("expectation.json");
    fs::write(
        &expectation_path,
        clew::canonical::bytes(&expectation).unwrap(),
    )
    .unwrap();
    let input_digest = repo.input_digest().unwrap();
    drop(repo);
    measured(
        "expect",
        &[
            "docs",
            "evidence",
            "expect",
            "--root",
            consumer.to_str().unwrap(),
            "--input",
            expectation_path.to_str().unwrap(),
            "--expected-input-digest",
            &input_digest,
        ],
        0,
    );
    let imported = measured(
        "import",
        &[
            "docs",
            "evidence",
            "import",
            "--root",
            consumer.to_str().unwrap(),
            "--input",
            package.to_str().unwrap(),
        ],
        0,
    );
    assert_eq!(imported["status"], "IMPORTED_RETAINED_SNAPSHOT");
    let replay = measured(
        "reimport",
        &[
            "docs",
            "evidence",
            "import",
            "--root",
            consumer.to_str().unwrap(),
            "--input",
            package.to_str().unwrap(),
        ],
        0,
    );
    assert_eq!(replay["status"], "RETAINED_ALREADY_IMPORTED");
    let checked = measured(
        "historical-check",
        &["docs", "check", "--root", consumer.to_str().unwrap()],
        3,
    );
    let repo = Repository::open(&consumer).unwrap();
    let (digest, size) = checked["snapshot"]
        .as_str()
        .unwrap()
        .rsplit_once('/')
        .unwrap();
    let reference = cache::ObjectRef::new(
        clew::documentation::check::CHECK_MANIFEST_SCHEMA.into(),
        digest.into(),
        size.parse().unwrap(),
    );
    let saved: clew::documentation::check::CheckManifest =
        cache::get_json(&repo, &reference, 128 * 1024 * 1024)
            .unwrap()
            .unwrap();
    assert_eq!(
        saved.service_manifests["orders"].cacheability,
        cache::NON_CACHEABLE
    );
    fs::write(output.join("RESULT.json"),serde_json::to_vec_pretty(&json!({"status":"PASS_NATIVE_AUTHENTICATED_CLI","frozenOracle":oracle,"export":export,"import":imported,"reimport":replay,"historicalCheck":checked,"cacheability":cache::NON_CACHEABLE,"treatment":"Fresh authenticated development CLI processes; no capture/model. Full independent typed oracle gate remains separate."})).unwrap()).unwrap();
}
