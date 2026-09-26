//! Public CLI regressions for bounded reads of immutable retained SOURCE rows.
#![cfg(unix)]

#[path = "support/documentation.rs"]
mod support;

use clew::documentation::{
    check::Check,
    model::Source,
    store::Repository,
    work::{self, SourcePartRequest},
};
use serde_json::{Value, json};
use std::os::unix::fs::PermissionsExt;
use std::{collections::BTreeMap, fs};
use support::{Fixture, read};

const SOURCE_PART_REQUEST_SCHEMA: &str = "codeclew-documentation-source-part-request/1.0";

fn prepare_work_for(
    f: &Fixture,
    max_bytes: usize,
    entrypoint: Option<&str>,
    snapshot: Option<&str>,
) -> (String, String) {
    prepare_work_for_source(f, max_bytes, entrypoint, snapshot, None)
}

fn prepare_work_for_source(
    f: &Fixture,
    max_bytes: usize,
    entrypoint: Option<&str>,
    snapshot: Option<&str>,
    source_id: Option<&str>,
) -> (String, String) {
    let mut request_value = json!({
        "schema":"codeclew-documentation-work-request/1.0",
        "audience":"Service maintainers",
        "maxItems":100,
        "maxBytes":max_bytes
    });
    if let Some(entrypoint) = entrypoint {
        request_value["entrypoint"] = json!(entrypoint);
    }
    let request = f.input("source-part-work-request.json", &request_value);
    let mut args = vec![
        "docs",
        "work",
        "prepare",
        "--subject",
        "service:orders",
        "--input",
        request.to_str().unwrap(),
    ];
    if let Some(snapshot) = snapshot {
        args.extend(["--snapshot", snapshot]);
    }
    let prepared = f.ok(&args);
    let work = prepared["work"].as_str().unwrap().to_owned();
    let frozen = read(f.docs.join(format!(".codeclew/work/{work}/work.json")));
    let reference = frozen["handles"]
        .as_object()
        .unwrap()
        .iter()
        .find(|(_, handle)| {
            handle["kind"] == "SOURCE" && source_id.is_none_or(|wanted| handle["id"] == wanted)
        })
        .map(|(reference, _)| reference.clone())
        .expect("retained Work contains a SOURCE handle");
    (work, reference)
}

fn prepare_work(f: &Fixture, max_bytes: usize) -> (String, String) {
    prepare_work_for(f, max_bytes, None, None)
}

fn add_synthetic_source(
    f: &Fixture,
    checked: Check,
    requested_bytes: usize,
) -> (Check, String, Source, String) {
    let line = format!("// retained source {} {}\n", '\u{e9}', '\u{1f680}');
    let mut text = String::with_capacity(requested_bytes + line.len());
    while text.len() < requested_bytes {
        text.push_str(&line);
    }
    add_synthetic_source_text(f, checked, text)
}

fn add_synthetic_source_text(
    f: &Fixture,
    mut checked: Check,
    text: String,
) -> (Check, String, Source, String) {
    let id = "orders:synthetic-large-source".to_owned();
    let source = Source {
        id: id.clone(),
        service: "orders".into(),
        revision: checked.services["orders"].revision.clone(),
        file: "Generated.java".into(),
        start_line: 1,
        end_line: text.lines().count().max(1) as u64,
        text_digest: clew::canonical::hash_bytes(text.as_bytes()),
        text,
        evidence_digest: format!("sha256:{}", "e".repeat(64)),
        authority: "EXACT_SNAPSHOT_TEXT".into(),
        occurrence: None,
        url: None,
    };
    let linked_dependency = {
        let evidence = checked.services.get_mut("orders").unwrap();
        evidence.sources.insert(id.clone(), source.clone());
        let entrypoint = evidence
            .entrypoints
            .iter_mut()
            .find(|entrypoint| entrypoint.symbol.contains("reserve"))
            .expect("fixture reserve entrypoint");
        entrypoint.source_ids.push(id.clone());
        entrypoint.source_ids.sort();
        entrypoint.source_ids.dedup();
        let dependency_id = entrypoint
            .dependency_ids
            .first()
            .cloned()
            .expect("fixture reserve dependency");
        let observation = evidence
            .observations
            .get_mut(&dependency_id)
            .expect("entrypoint dependency observation");
        observation.source_ids.push(id.clone());
        observation.source_ids.sort();
        observation.source_ids.dedup();
        dependency_id
    };
    let dependency = checked
        .dependencies
        .get_mut(&linked_dependency)
        .expect("check dependency mirrors service observation");
    dependency.source_ids.push(id.clone());
    dependency.source_ids.sort();
    dependency.source_ids.dedup();
    let repo = Repository::open(&f.docs).unwrap();
    let snapshot = checked.save_snapshot(&repo).unwrap();
    (checked, snapshot, source, id)
}

fn request_value(reference: &str, cursor: Option<&str>) -> SourcePartRequest {
    SourcePartRequest {
        schema: SOURCE_PART_REQUEST_SCHEMA.into(),
        reference: reference.into(),
        cursor: cursor.map(str::to_owned),
    }
}

fn submit_proposal(f: &Fixture, work_id: &str, proposal: &Value, name: &str) -> (i32, Value) {
    let input = f.input(name, proposal);
    f.run(&[
        "docs",
        "proposal",
        "submit",
        "--work",
        work_id,
        "--input",
        input.to_str().unwrap(),
    ])
}

fn read_default_page_chain(f: &Fixture, work_id: &str) -> Vec<Value> {
    let mut pages = Vec::new();
    let mut cursor: Option<String> = None;
    loop {
        let selection = cursor
            .as_ref()
            .map_or_else(|| json!({}), |cursor| json!({"cursor":cursor}));
        let input = f.input("source-part-default-selection.json", &selection);
        let (code, page) = f.run(&[
            "docs",
            "work",
            "read",
            "--work",
            work_id,
            "--input",
            input.to_str().unwrap(),
        ]);
        assert_eq!(code, 0, "{page}");
        cursor = page["nextCursor"].as_str().map(str::to_owned);
        pages.push(page);
        if cursor.is_none() {
            return pages;
        }
    }
}

fn has_diagnostic(result: &Value, code: &str) -> bool {
    result["items"].as_array().is_some_and(|items| {
        items
            .iter()
            .any(|item| item["kind"] == "DIAGNOSTIC" && item["record"]["code"] == code)
    })
}

fn diagnostic_codes(result: &Value) -> Vec<String> {
    result["items"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|item| item["kind"] == "DIAGNOSTIC")
        .filter_map(|item| item["record"]["code"].as_str().map(str::to_owned))
        .collect()
}

#[cfg(target_os = "macos")]
fn execution_config(driver: &std::path::Path, account: &str) -> Value {
    let role = json!({
        "adapter":"macos-seatbelt-stdio/1.0",
        "model":"source-part-fixture",
        "usageAuthority":"MAXIMUM_ONLY",
        "command":[driver.to_str().unwrap()],
        "runtimeReads":[],
        "environment":[],
        "network":false,
        "cap":{
            "maximum":{"inputTokens":1000000,"outputTokens":100000,"costUnits":100},
            "overheadInputTokens":1,
            "timeoutMs":5000,
            "outputBytes":65536
        }
    });
    json!({
        "schema":"codeclew-documentation-execution/1.0",
        "author":role,
        "reviewer":role,
        "authorCalls":2,
        "reviewerCalls":2,
        "fallback":null,
        "fallbackCalls":0,
        "repairAttempts":1,
        "expansions":0,
        "budget":{
            "account":account,
            "costUnit":"fixture-unit",
            "ceiling":{"inputTokens":10000000,"outputTokens":1000000,"costUnits":1000},
            "stopLoss":{"inputTokens":9999999,"outputTokens":999999,"costUnits":999}
        }
    })
}

fn part_input(f: &Fixture, reference: &str, cursor: Option<&str>) -> std::path::PathBuf {
    let mut request = json!({
        "schema":"codeclew-documentation-source-part-request/1.0",
        "reference":reference
    });
    if let Some(cursor) = cursor {
        request["cursor"] = json!(cursor);
    }
    f.input("source-part-request.json", &request)
}

fn write_optional_artifact(name: &str, bytes: &[u8]) {
    let Some(root) = std::env::var_os("CODECLEW_SOURCE_PART_ARTIFACT_DIR") else {
        return;
    };
    let path = std::path::PathBuf::from(root).join(name);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, bytes).unwrap();
}

fn write_optional_json_artifact(name: &str, value: &Value) {
    let mut bytes = clew::canonical::bytes(value).unwrap();
    bytes.push(b'\n');
    write_optional_artifact(name, &bytes);
}

fn job_report_count(path: &std::path::Path) -> usize {
    fs::read_dir(path)
        .map(|entries| {
            entries
                .flatten()
                .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "json"))
                .count()
        })
        .unwrap_or(0)
}

fn author_attempt_count(path: &std::path::Path) -> usize {
    fs::read_dir(path)
        .map(|entries| {
            entries
                .flatten()
                .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "json"))
                .map(|entry| {
                    fs::read(entry.path())
                        .ok()
                        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
                        .and_then(|report| report["attempts"].as_array().map(Vec::len))
                        .unwrap_or(0)
                })
                .sum()
        })
        .unwrap_or(0)
}

#[test]
fn retained_source_part_cli_returns_bounded_recorded_utf8_bytes_without_acquisition() {
    let f = Fixture::new();
    let source_repo = f.service("orders");
    let checked = f.checked();
    let latest_path = f.docs.join(".codeclew/cache/latest-check.json");
    let latest_before = fs::read(&latest_path).unwrap();
    assert!(serde_json::from_slice::<Value>(&latest_before).is_ok());
    let (work, reference) = prepare_work(&f, 2_048);
    assert_eq!(fs::read(&latest_path).unwrap(), latest_before);
    let frozen = read(f.docs.join(format!(".codeclew/work/{work}/work.json")));
    let source_id = frozen["handles"][&reference]["id"].as_str().unwrap();
    let source = checked
        .services
        .values()
        .find_map(|service| service.sources.get(source_id))
        .expect("prepared source handle belongs to the captured snapshot");
    let work_manifest_path = f.docs.join(format!(".codeclew/work/{work}/work.json"));
    let work_manifest_before = fs::read(&work_manifest_path).unwrap();
    let repo = Repository::open(&f.docs).unwrap();
    let retained_check_digest_before = clew::canonical::hash(
        &Check::load_snapshot(&repo, frozen["snapshot"].as_str().unwrap()).unwrap(),
    )
    .unwrap();
    let source_offline = source_repo.with_extension("offline");
    fs::rename(&source_repo, &source_offline).unwrap();
    let poisoned_latest = b"not a valid latest-check pointer\n";
    fs::write(&latest_path, poisoned_latest).unwrap();
    let inventory_before =
        clew::canonical::bytes(&clew::documentation::cache::inventory(&repo).unwrap()).unwrap();

    // Keep this process-independent acquisition trap ahead of the ambient PATH.
    let trap_dir = f.temp.path().join("acquisition-traps");
    fs::create_dir(&trap_dir).unwrap();
    let marker = f.temp.path().join("unexpected-acquisition");
    for executable in ["git", "java", "javac", "gradle", "mvn", "kotlinc"] {
        let path = trap_dir.join(executable);
        fs::write(
            &path,
            format!(
                "#!/bin/sh\nprintf '%s\\n' '{}' >> '{}'\nexit 91\n",
                executable,
                marker.display()
            ),
        )
        .unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
    }

    let input = part_input(&f, &reference, None);
    let output = f.run_raw_with_path(
        &[
            "docs",
            "work",
            "read-part",
            "--work",
            &work,
            "--input",
            input.to_str().unwrap(),
        ],
        &trap_dir,
    );
    assert!(
        output.status.success(),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stdout.last(), Some(&b'\n'));
    assert!(output.stdout.len() <= 2_048);
    let response: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(response["schema"], "codeclew-documentation-source-part/1.0");
    assert_eq!(response["work"], work);
    assert_eq!(response["reference"], reference);
    assert_eq!(response["startByte"], 0);
    assert_eq!(response["text"], source.text);
    assert_eq!(response["source"]["textDigest"], source.text_digest);
    assert!(response["source"].get("text").is_none());
    let stderr = String::from_utf8_lossy(&output.stderr);
    let progress: Vec<Value> = stderr
        .lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect();
    let acquisition_phases = [
        "ACQUIRE_SERVICE_EVIDENCE",
        "ACQUIRE_SYNTAX_EVIDENCE",
        "ACQUIRE_COMPILER_EVIDENCE",
        "ENSURE_COMPILER_GENERATION",
    ];
    for phase in acquisition_phases {
        assert_eq!(
            progress
                .iter()
                .filter(|event| event["phase"] == phase && event["event"] == "STARTED")
                .count(),
            0,
            "unexpected {phase} phase in {stderr}"
        );
    }
    for retained_phase in ["LOAD_WORK_RECORD", "LOAD_RETAINED_SNAPSHOT"] {
        assert!(
            progress
                .iter()
                .any(|event| event["phase"] == retained_phase && event["event"] == "STARTED"),
            "missing {retained_phase} phase in {stderr}"
        );
    }
    assert!(!marker.exists(), "an acquisition executable was invoked");
    assert_eq!(fs::read(latest_path).unwrap(), poisoned_latest);
    assert!(source_offline.exists());
    let work_manifest_after = fs::read(&work_manifest_path).unwrap();
    assert_eq!(work_manifest_after, work_manifest_before);
    let retained_check_digest_after = clew::canonical::hash(
        &Check::load_snapshot(&repo, frozen["snapshot"].as_str().unwrap()).unwrap(),
    )
    .unwrap();
    let inventory_after =
        clew::canonical::bytes(&clew::documentation::cache::inventory(&repo).unwrap()).unwrap();
    assert_eq!(retained_check_digest_after, retained_check_digest_before);
    assert_eq!(inventory_after, inventory_before);
    write_optional_json_artifact(
        "offline-pinned-read.json",
        &json!({
            "schema":"codeclew-source-part-offline-evidence/1.0",
            "responseRecordDigest":response["recordDigest"],
            "textDigest":response["source"]["textDigest"],
            "workManifestStable":true,
            "workManifestDigestBefore":clew::canonical::hash_bytes(&work_manifest_before),
            "workManifestDigestAfter":clew::canonical::hash_bytes(&work_manifest_after),
            "pinnedCheckDigestBefore":retained_check_digest_before,
            "pinnedCheckDigestAfter":retained_check_digest_after,
            "cacheInventoryDigestBefore":clew::canonical::hash_bytes(&inventory_before),
            "cacheInventoryDigestAfter":clew::canonical::hash_bytes(&inventory_after),
            "latestPointerFollowed":false,
            "sourceRepoOffline":source_offline.exists(),
            "acquisitionExecutableTrapAttempts":0
        }),
    );

    let ledger = read(f.docs.join(format!(".codeclew/work/{work}/reads.json")));
    let receipts = ledger["sourcePartReceipts"].as_object().unwrap();
    assert_eq!(receipts.len(), 1);
    let receipt = receipts.values().next().unwrap();
    assert_eq!(receipt["reference"], reference);
    assert_eq!(receipt["startByte"], 0);
    assert_eq!(receipt["endByte"], response["endByte"]);
    assert_eq!(receipt["receiptDigest"], response["receiptDigest"]);
}

#[test]
fn source_part_requests_are_strict_work_scoped_retryable_and_concurrency_safe() {
    let f = Fixture::new();
    f.service("orders");
    let checked = f.checked();
    let (captured_check, snapshot, _source, source_id) =
        add_synthetic_source(&f, checked, 12 * 1024);
    let (work_a, reference_a) =
        prepare_work_for_source(&f, 2_048, None, Some(&snapshot), Some(&source_id));
    let (work_b, reference_b) =
        prepare_work_for_source(&f, 40_960, None, Some(&snapshot), Some(&source_id));
    assert_ne!(work_a, work_b);

    let first_request = json!({
        "schema":SOURCE_PART_REQUEST_SCHEMA,
        "reference":reference_a
    });
    let first_input = f.input("source-part-first-request.json", &first_request);
    let first_args = [
        "docs",
        "work",
        "read-part",
        "--work",
        &work_a,
        "--input",
        first_input.to_str().unwrap(),
    ];
    let first = f.run_raw(&first_args);
    assert!(
        first.status.success(),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    let first_response: Value = serde_json::from_slice(&first.stdout).unwrap();
    let cursor = first_response["nextCursor"].as_str().unwrap().to_owned();
    let repo = Repository::open(&f.docs).unwrap();
    let state_a_path = f.docs.join(format!(".codeclew/work/{work_a}/reads.json"));
    let state_a = work::read_state(&repo, &work_a).unwrap();
    assert_eq!(state_a.source_part_receipts.len(), 1);
    let first_receipt_bytes = fs::read(&state_a_path).unwrap();

    // A separate process replay of the exact request returns the same bytes
    // and does not append a duplicate receipt.
    let retry_first = f.run_raw(&first_args);
    assert!(retry_first.status.success());
    assert_eq!(retry_first.stdout, first.stdout);
    assert_eq!(fs::read(&state_a_path).unwrap(), first_receipt_bytes);

    let unknown_field = f.input(
        "source-part-unknown-field.json",
        &json!({
            "schema":SOURCE_PART_REQUEST_SCHEMA,
            "reference":reference_a,
            "unexpected":true
        }),
    );
    let strict_error = f.run_raw(&[
        "docs",
        "work",
        "read-part",
        "--work",
        &work_a,
        "--input",
        unknown_field.to_str().unwrap(),
    ]);
    assert!(!strict_error.status.success());
    let wrong_schema = f.input(
        "source-part-wrong-schema.json",
        &json!({
            "schema":"codeclew-documentation-source-part-request/0.9",
            "reference":reference_a
        }),
    );
    let schema_error = f.run_raw(&[
        "docs",
        "work",
        "read-part",
        "--work",
        &work_a,
        "--input",
        wrong_schema.to_str().unwrap(),
    ]);
    assert!(!schema_error.status.success());
    assert_eq!(fs::read(&state_a_path).unwrap(), first_receipt_bytes);

    let frozen_a = read(f.docs.join(format!(".codeclew/work/{work_a}/work.json")));
    let dependency_reference = frozen_a["handles"]
        .as_object()
        .unwrap()
        .iter()
        .find(|(_, handle)| handle["kind"] == "DEPENDENCY")
        .map(|(reference, _)| reference.clone())
        .expect("Work contains a non-SOURCE handle");
    for (case, invalid_reference) in [
        ("unknown-reference", "source:does-not-exist".to_owned()),
        ("dependency-reference", dependency_reference),
    ] {
        let request = f.input(
            &format!("source-part-{case}.json"),
            &json!({
                "schema":SOURCE_PART_REQUEST_SCHEMA,
                "reference":invalid_reference
            }),
        );
        let output = f.run_raw(&[
            "docs",
            "work",
            "read-part",
            "--work",
            &work_a,
            "--input",
            request.to_str().unwrap(),
        ]);
        assert!(!output.status.success(), "accepted {case}");
        assert_eq!(fs::read(&state_a_path).unwrap(), first_receipt_bytes);
    }

    let foreign_cursor = f.input(
        "source-part-foreign-cursor.json",
        &json!({
            "schema":SOURCE_PART_REQUEST_SCHEMA,
            "reference":reference_b,
            "cursor":cursor
        }),
    );
    let cursor_error = f.run_raw(&[
        "docs",
        "work",
        "read-part",
        "--work",
        &work_b,
        "--input",
        foreign_cursor.to_str().unwrap(),
    ]);
    assert!(!cursor_error.status.success());
    assert!(
        format!(
            "{}{}",
            String::from_utf8_lossy(&cursor_error.stdout),
            String::from_utf8_lossy(&cursor_error.stderr)
        )
        .contains("source-part cursor belongs to another Work"),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&cursor_error.stdout),
        String::from_utf8_lossy(&cursor_error.stderr)
    );
    assert!(
        work::read_state(&repo, &work_b)
            .unwrap()
            .source_part_receipts
            .is_empty()
    );

    // Two readers race on different SOURCE rows. The repository's existing
    // nonblocking writer lock may reject one; retrying it must preserve both
    // new receipts plus the earlier receipt.
    let other_source_id = captured_check.services["orders"]
        .sources
        .keys()
        .find(|id| id.as_str() != source_id.as_str())
        .expect("fixture has a second retained SOURCE")
        .clone();
    let other_reference = frozen_a["handles"]
        .as_object()
        .unwrap()
        .iter()
        .find(|(_, handle)| handle["kind"] == "SOURCE" && handle["id"] == other_source_id)
        .map(|(reference, _)| reference.clone())
        .expect("Work contains the second SOURCE handle");
    let next_source_request = f.input(
        "source-part-concurrent-next-source.json",
        &json!({
            "schema":SOURCE_PART_REQUEST_SCHEMA,
            "reference":reference_a,
            "cursor":cursor
        }),
    );
    let other_source_request = f.input(
        "source-part-concurrent-other-source.json",
        &json!({
            "schema":SOURCE_PART_REQUEST_SCHEMA,
            "reference":other_reference
        }),
    );
    let next_source_args = [
        "docs",
        "work",
        "read-part",
        "--work",
        &work_a,
        "--input",
        next_source_request.to_str().unwrap(),
    ];
    let other_source_args = [
        "docs",
        "work",
        "read-part",
        "--work",
        &work_a,
        "--input",
        other_source_request.to_str().unwrap(),
    ];
    let (next_source_output, other_source_output) = std::thread::scope(|scope| {
        let next_source = scope.spawn(|| f.run_raw(&next_source_args));
        let other_source = scope.spawn(|| f.run_raw(&other_source_args));
        (next_source.join().unwrap(), other_source.join().unwrap())
    });
    let requests = [
        (next_source_output, &reference_a, &next_source_args),
        (other_source_output, &other_reference, &other_source_args),
    ];
    let mut completed = Vec::new();
    for (output, expected_reference, args) in requests {
        let output = if output.status.success() {
            output
        } else {
            let detail = format!(
                "{}{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            assert!(
                detail.contains("documentation writer lock exists"),
                "{detail}"
            );
            f.run_raw(args)
        };
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let response: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(response["reference"], *expected_reference);
        completed.push(output.stdout);
    }
    let state_after_concurrency = work::read_state(&repo, &work_a).unwrap();
    assert_eq!(state_after_concurrency.source_part_receipts.len(), 3);
    let completed_references: std::collections::BTreeSet<_> = state_after_concurrency
        .source_part_receipts
        .values()
        .map(|receipt| receipt.reference.as_str())
        .collect();
    assert!(completed_references.contains(reference_a.as_str()));
    assert!(completed_references.contains(other_reference.as_str()));
    let replay_second_source = f.run_raw(&next_source_args);
    assert!(replay_second_source.status.success());
    assert_eq!(replay_second_source.stdout, completed[0]);
    let restarted_first = f.run_raw(&first_args);
    assert!(restarted_first.status.success());
    assert_eq!(restarted_first.stdout, first.stdout);
    assert_eq!(
        work::read_state(&repo, &work_a)
            .unwrap()
            .source_part_receipts
            .len(),
        3
    );
}

#[test]
fn no_progress_and_missing_retained_parent_fail_without_receipts_or_acquisition() {
    let f = Fixture::new();
    let source_repo = f.service("orders");
    let checked = f.checked();
    let (mut checked, _initial_snapshot, _source, source_id) =
        add_synthetic_source_text(&f, checked, String::new());
    let oversized_file = format!("generated/{}.java", "m".repeat(4096));
    checked
        .services
        .get_mut("orders")
        .unwrap()
        .sources
        .get_mut(&source_id)
        .unwrap()
        .file = oversized_file;
    let repo = Repository::open(&f.docs).unwrap();
    let snapshot = checked.save_snapshot(&repo).unwrap();
    let (work_id, reference) =
        prepare_work_for_source(&f, 2_048, None, Some(&snapshot), Some(&source_id));
    let work_manifest_path = f.docs.join(format!(".codeclew/work/{work_id}/work.json"));
    let work_manifest_before = fs::read(&work_manifest_path).unwrap();
    let reads_path = f.docs.join(format!(".codeclew/work/{work_id}/reads.json"));
    let reads_before = fs::read(&reads_path).ok();
    let pinned_digest_before =
        clew::canonical::hash(&Check::load_snapshot(&repo, &snapshot).unwrap()).unwrap();
    let inventory_before =
        clew::canonical::bytes(&&clew::documentation::cache::inventory(&repo).unwrap()).unwrap();

    let request = part_input(&f, &reference, None);
    let no_progress = f.run_raw(&[
        "docs",
        "work",
        "read-part",
        "--work",
        &work_id,
        "--input",
        request.to_str().unwrap(),
    ]);
    assert!(!no_progress.status.success());
    let no_progress_detail = format!(
        "{}{}",
        String::from_utf8_lossy(&no_progress.stdout),
        String::from_utf8_lossy(&no_progress.stderr)
    );
    assert!(
        no_progress_detail.contains("SOURCE_PART_NO_PROGRESS"),
        "{no_progress_detail}"
    );
    assert!(no_progress_detail.contains("prepare a new Work"));
    assert_eq!(fs::read(&work_manifest_path).unwrap(), work_manifest_before);
    assert_eq!(fs::read(&reads_path).ok(), reads_before);
    assert!(
        work::read_state(&repo, &work_id)
            .unwrap()
            .source_part_receipts
            .is_empty()
    );
    let pinned_digest_after =
        clew::canonical::hash(&Check::load_snapshot(&repo, &snapshot).unwrap()).unwrap();
    let inventory_after =
        clew::canonical::bytes(&clew::documentation::cache::inventory(&repo).unwrap()).unwrap();
    assert_eq!(pinned_digest_after, pinned_digest_before);
    assert_eq!(inventory_after, inventory_before);

    let retained = Check::load_snapshot(&repo, &snapshot).unwrap();
    let retained_manifest = retained.store_manifest(&repo).unwrap();
    let source_payload = retained_manifest.service_manifests["orders"]
        .sources
        .clone();
    let original_payload = clew::documentation::cache::get(
        &repo,
        &source_payload,
        clew::documentation::check::PORTABLE_CACHE_MAX_BYTES,
    )
    .unwrap()
    .expect("captured source payload exists before failure injection");
    let layout = read(f.docs.join(".codeclew/cache/object-layout.json"));
    let database_path = f.docs.join(layout["database"].as_str().unwrap());
    assert!(database_path.is_file());
    fs::rename(&source_repo, source_repo.with_extension("offline")).unwrap();
    let latest_path = f.docs.join(".codeclew/cache/latest-check.json");
    let poisoned_latest = b"missing retained parent must not follow latest\n";
    fs::write(&latest_path, poisoned_latest).unwrap();
    let trap_dir = f.temp.path().join("missing-parent-acquisition-traps");
    fs::create_dir(&trap_dir).unwrap();
    let marker = f.temp.path().join("missing-parent-unexpected-acquisition");
    for executable in ["git", "java", "javac", "gradle", "mvn", "kotlinc"] {
        let path = trap_dir.join(executable);
        fs::write(
            &path,
            format!(
                "#!/bin/sh\nprintf '%s\\n' '{}' >> '{}'\nexit 91\n",
                executable,
                marker.display()
            ),
        )
        .unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
    }
    let mut corrupt_payload = original_payload;
    corrupt_payload[0] ^= 1;
    let changed = rusqlite::Connection::open(&database_path)
        .unwrap()
        .execute(
            "UPDATE objects SET payload = ?1 WHERE digest = ?2",
            rusqlite::params![corrupt_payload.as_slice(), source_payload.digest],
        )
        .unwrap();
    assert_eq!(changed, 1);

    let run_parent_failure = || {
        f.run_raw_with_path(
            &[
                "docs",
                "work",
                "read-part",
                "--work",
                &work_id,
                "--input",
                request.to_str().unwrap(),
            ],
            &trap_dir,
        )
    };
    let corrupt_parent = run_parent_failure();
    assert!(!corrupt_parent.status.success());
    let corrupt_detail = format!(
        "{}{}",
        String::from_utf8_lossy(&corrupt_parent.stdout),
        String::from_utf8_lossy(&corrupt_parent.stderr)
    );
    assert!(
        corrupt_detail.contains("digest")
            || corrupt_detail.contains("corrupt")
            || corrupt_detail.contains("invalid"),
        "{corrupt_detail}"
    );
    let corrupt_progress: Vec<Value> = String::from_utf8_lossy(&corrupt_parent.stderr)
        .lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect();
    assert!(
        corrupt_progress
            .iter()
            .any(|event| { event["phase"] == "LOAD_WORK_RECORD" && event["event"] == "STARTED" })
    );
    assert!(corrupt_progress.iter().any(|event| {
        event["phase"] == "LOAD_RETAINED_SNAPSHOT" && event["event"] == "STARTED"
    }));
    for phase in [
        "ACQUIRE_SERVICE_EVIDENCE",
        "ACQUIRE_SYNTAX_EVIDENCE",
        "ACQUIRE_COMPILER_EVIDENCE",
        "ENSURE_COMPILER_GENERATION",
    ] {
        assert!(
            !corrupt_progress
                .iter()
                .any(|event| event["phase"] == phase && event["event"] == "STARTED")
        );
    }
    assert!(!marker.exists());

    let deleted = rusqlite::Connection::open(&database_path)
        .unwrap()
        .execute(
            "DELETE FROM objects WHERE digest = ?1",
            rusqlite::params![source_payload.digest],
        )
        .unwrap();
    assert_eq!(deleted, 1);
    let inventory_before_missing_read =
        clew::canonical::bytes(&clew::documentation::cache::inventory(&repo).unwrap()).unwrap();
    let missing_parent = f.run_raw_with_path(
        &[
            "docs",
            "work",
            "read-part",
            "--work",
            &work_id,
            "--input",
            request.to_str().unwrap(),
        ],
        &trap_dir,
    );
    assert!(!missing_parent.status.success());
    let missing_detail = format!(
        "{}{}",
        String::from_utf8_lossy(&missing_parent.stdout),
        String::from_utf8_lossy(&missing_parent.stderr)
    );
    assert!(
        missing_detail.contains("missing")
            || missing_detail.contains("unavailable")
            || missing_detail.contains("object reference"),
        "{missing_detail}"
    );
    let progress: Vec<Value> = String::from_utf8_lossy(&missing_parent.stderr)
        .lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect();
    assert!(
        progress
            .iter()
            .any(|event| { event["phase"] == "LOAD_WORK_RECORD" && event["event"] == "STARTED" })
    );
    assert!(progress.iter().any(|event| {
        event["phase"] == "LOAD_RETAINED_SNAPSHOT" && event["event"] == "STARTED"
    }));
    for phase in [
        "ACQUIRE_SERVICE_EVIDENCE",
        "ACQUIRE_SYNTAX_EVIDENCE",
        "ACQUIRE_COMPILER_EVIDENCE",
        "ENSURE_COMPILER_GENERATION",
    ] {
        assert!(
            !progress
                .iter()
                .any(|event| event["phase"] == phase && event["event"] == "STARTED")
        );
    }
    assert!(!marker.exists());
    assert_eq!(fs::read(&latest_path).unwrap(), poisoned_latest);
    assert_eq!(fs::read(&work_manifest_path).unwrap(), work_manifest_before);
    assert_eq!(fs::read(&reads_path).ok(), reads_before);
    let inventory_after_missing_read =
        clew::canonical::bytes(&clew::documentation::cache::inventory(&repo).unwrap()).unwrap();
    assert_eq!(inventory_after_missing_read, inventory_before_missing_read);
    write_optional_json_artifact(
        "no-progress-and-parent-failure.json",
        &json!({
            "schema":"codeclew-source-part-failure-evidence/1.0",
            "noProgressMarker":"SOURCE_PART_NO_PROGRESS",
            "noProgressReceiptRecorded":false,
            "workManifestStableAfterNoProgress":true,
            "cacheInventoryStableAfterNoProgress":true,
            "pinnedCheckDigestStableAfterNoProgress":pinned_digest_after,
            "corruptRetainedSourcePayloadFailure":true,
            "corruptPayloadLoadWorkRecordStarted":true,
            "corruptPayloadLoadRetainedSnapshotStarted":true,
            "corruptPayloadAcquisitionStartedTotal":0,
            "corruptPayloadExecutableTrapAttempts":0,
            "missingRetainedSourcePayloadFailure":true,
            "missingPayloadLoadWorkRecordStarted":true,
            "missingPayloadLoadRetainedSnapshotStarted":true,
            "missingPayloadAcquisitionStarted":false,
            "missingPayloadExecutableTrapAttempts":0,
            "cacheInventoryStableAfterMissingPayloadFailure":true,
            "latestPointerFollowed":false,
            "workManifestStableAfterParentFailure":true
        }),
    );
}

#[test]
fn source_part_ledger_bound_rejection_is_atomic() {
    const MAX_LEDGER_BYTES: usize = 16 * 1024 * 1024;
    let f = Fixture::new();
    f.service("orders");
    let checked = f.checked();
    let (_checked, snapshot, _source, source_id) =
        add_synthetic_source_text(&f, checked, "small retained source\n".to_owned());
    let (work_id, reference) =
        prepare_work_for_source(&f, 2_048, None, Some(&snapshot), Some(&source_id));
    let repo = Repository::open(&f.docs).unwrap();
    let request = part_input(&f, &reference, None);
    let args = [
        "docs",
        "work",
        "read-part",
        "--work",
        &work_id,
        "--input",
        request.to_str().unwrap(),
    ];
    let first = f.run_raw(&args);
    assert!(first.status.success());
    let mut state = work::read_state(&repo, &work_id).unwrap();
    let receipt_key = state
        .source_part_receipts
        .keys()
        .next()
        .expect("first part records one receipt")
        .clone();
    state
        .source_part_receipts
        .get_mut(&receipt_key)
        .unwrap()
        .end_byte += 1;
    let conflicting = clew::canonical::bytes(&state).unwrap();
    let reads_path = f.docs.join(format!(".codeclew/work/{work_id}/reads.json"));
    fs::write(&reads_path, &conflicting).unwrap();
    let conflict = f.run_raw(&args);
    assert!(!conflict.status.success());
    let conflict_detail = format!(
        "{}{}",
        String::from_utf8_lossy(&conflict.stdout),
        String::from_utf8_lossy(&conflict.stderr)
    );
    assert!(
        conflict_detail.contains("source-part receipt identity conflicts with saved evidence"),
        "{conflict_detail}"
    );
    assert_eq!(fs::read(&reads_path).unwrap(), conflicting);
    state.source_part_receipts.clear();
    state.receipts.insert(
        "ledger-padding".into(),
        work::ReadReceipt {
            selection: work::Selection::default(),
            result_digest: String::new(),
            supplied: Vec::new(),
            membership_digest: "sha256:ledger-padding".into(),
            omitted: Vec::new(),
            next_cursor: None,
        },
    );
    let expected_ordinary_receipt_count = state.receipts.len();
    let base_bytes = clew::canonical::bytes(&state).unwrap().len();
    let filler_bytes = MAX_LEDGER_BYTES - 32 - base_bytes;
    state
        .receipts
        .get_mut("ledger-padding")
        .unwrap()
        .result_digest = "x".repeat(filler_bytes);
    let before = clew::canonical::bytes(&state).unwrap();
    assert_eq!(before.len(), MAX_LEDGER_BYTES - 32);
    fs::write(&reads_path, &before).unwrap();

    let output = f.run_raw(&[
        "docs",
        "work",
        "read-part",
        "--work",
        &work_id,
        "--input",
        request.to_str().unwrap(),
    ]);
    assert!(!output.status.success());
    let detail = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        detail.contains("work read ledger exceeds its bound"),
        "{detail}"
    );
    assert_eq!(fs::read(&reads_path).unwrap(), before);
    let after = work::read_state(&repo, &work_id).unwrap();
    assert!(after.source_part_receipts.is_empty());
    assert_eq!(after.receipts.len(), expected_ordinary_receipt_count);
    write_optional_json_artifact(
        "ledger-bound-atomicity.json",
        &json!({
            "schema":"codeclew-source-part-ledger-limit-evidence/1.0",
            "priorLedgerBytes":before.len(),
            "ledgerLimitBytes":MAX_LEDGER_BYTES,
            "rejection":"work read ledger exceeds its bound",
            "conflictingReceiptIdentityRejectedWithoutMutation":true,
            "ledgerUnchangedAfterConflictingRetry":true,
            "ledgerBytesUnchanged":true,
            "newPartReceiptRecorded":false
        }),
    );
}

#[test]
fn source_part_raw_cli_matrix_stays_within_budgets_and_reconstructs_records() {
    let ascii = "A".repeat(8 * 1024);
    let mixed_piece = format!(
        "quote: \" backslash: \\\\ newline:\n{} {}\n",
        '\u{0440}', '\u{1f680}'
    );
    let mut mixed = String::with_capacity(12 * 1024 + mixed_piece.len());
    while mixed.len() < 12 * 1024 {
        mixed.push_str(&mixed_piece);
    }
    let long_line = "L".repeat(64 * 1024);
    let empty = String::new();
    let mut small = String::from("small source");
    small.push_str(&".".repeat(32 - small.len()));
    let one_mib = "Z".repeat(1024 * 1024);
    let cases = [
        ("ascii-8k", ascii, vec![2_048usize, 40_960, 49_152]),
        (
            "mixed-unicode-escaping-12k",
            mixed,
            vec![2_048usize, 40_960, 49_152],
        ),
        ("long-line-64k", long_line, vec![2_048usize, 40_960, 49_152]),
        ("empty", empty, vec![2_048usize, 40_960, 49_152]),
        ("small-32b", small, vec![2_048usize, 40_960, 49_152]),
        ("ascii-1m", one_mib, vec![49_152usize]),
    ];
    let trap_names = ["git", "java", "javac", "gradle", "mvn", "kotlinc"];
    let acquisition_phases = [
        "ACQUIRE_SERVICE_EVIDENCE",
        "ACQUIRE_SYNTAX_EVIDENCE",
        "ACQUIRE_COMPILER_EVIDENCE",
        "ENSURE_COMPILER_GENERATION",
    ];
    let mut results = Vec::new();

    for (case_name, text, budgets) in cases {
        let f = Fixture::new();
        f.service("orders");
        let checked = f.checked();
        let (_checked, snapshot, source, source_id) = add_synthetic_source_text(&f, checked, text);
        let canonical_source_bytes = clew::canonical::bytes(&source).unwrap().len();
        let expected_record_digest = clew::canonical::hash(&source).unwrap();
        let repo = Repository::open(&f.docs).unwrap();
        let trap_dir = f.temp.path().join("matrix-acquisition-traps");
        fs::create_dir(&trap_dir).unwrap();
        let marker = f.temp.path().join("matrix-unexpected-acquisition");
        for executable in trap_names {
            let path = trap_dir.join(executable);
            fs::write(
                &path,
                format!(
                    "#!/bin/sh\nprintf '%s\\n' '{}' >> '{}'\nexit 91\n",
                    executable,
                    marker.display()
                ),
            )
            .unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
        }

        for max_bytes in budgets {
            let (work_id, reference) =
                prepare_work_for_source(&f, max_bytes, None, Some(&snapshot), Some(&source_id));
            let ledger_path = f.docs.join(format!(".codeclew/work/{work_id}/reads.json"));
            let ledger_before = fs::read(&ledger_path).map_or(0, |bytes| bytes.len());
            let jobs_dir = f.docs.join(".codeclew/jobs");
            let jobs_before = job_report_count(&jobs_dir);
            let attempts_before = author_attempt_count(&jobs_dir);
            let mut cursor = None;
            let mut offset = 0usize;
            let mut rebuilt = String::with_capacity(source.text.len());
            let retain_sample = std::env::var_os("CODECLEW_SOURCE_PART_ARTIFACT_DIR").is_some()
                && ((case_name == "ascii-8k" && max_bytes == 2_048)
                    || (case_name == "ascii-1m" && max_bytes == 49_152));
            let mut samples: Vec<(Vec<u8>, Vec<u8>)> = Vec::with_capacity(2);
            let mut sum_stdout_bytes = 0usize;
            let mut max_stdout_bytes = 0usize;
            let mut parts = 0usize;
            let mut load_work_started = 0usize;
            let mut load_snapshot_started = 0usize;
            let mut acquisition_counts = BTreeMap::from([
                ("ACQUIRE_SERVICE_EVIDENCE".to_owned(), 0usize),
                ("ACQUIRE_SYNTAX_EVIDENCE".to_owned(), 0usize),
                ("ACQUIRE_COMPILER_EVIDENCE".to_owned(), 0usize),
                ("ENSURE_COMPILER_GENERATION".to_owned(), 0usize),
            ]);
            let reconstructed_record_digest = loop {
                let request = request_value(&reference, cursor.as_deref());
                let request_value_json = serde_json::to_value(&request).unwrap();
                let request_path = f.input("source-part-matrix-request.json", &request_value_json);
                let raw = f.run_raw_with_path(
                    &[
                        "docs",
                        "work",
                        "read-part",
                        "--work",
                        &work_id,
                        "--input",
                        request_path.to_str().unwrap(),
                    ],
                    &trap_dir,
                );
                assert!(
                    raw.status.success(),
                    "case={case_name} maxBytes={max_bytes} stdout={} stderr={}",
                    String::from_utf8_lossy(&raw.stdout),
                    String::from_utf8_lossy(&raw.stderr)
                );
                assert_eq!(raw.stdout.last(), Some(&b'\n'));
                assert!(
                    raw.stdout.len() <= max_bytes,
                    "case={case_name} maxBytes={max_bytes} stdout={}",
                    raw.stdout.len()
                );
                let response: Value = serde_json::from_slice(&raw.stdout).unwrap();
                let canonical_line = clew::canonical::bytes(&response).unwrap();
                assert_eq!(raw.stdout, [canonical_line.as_slice(), b"\n"].concat());
                assert_eq!(response["schema"], "codeclew-documentation-source-part/1.0");
                assert_eq!(response["sourceId"], source_id);
                assert_eq!(response["recordDigest"], expected_record_digest);
                assert_eq!(response["startByte"].as_u64().unwrap() as usize, offset);
                let end = response["endByte"].as_u64().unwrap() as usize;
                assert!(source.text.is_char_boundary(offset));
                assert!(source.text.is_char_boundary(end));
                let fragment = response["text"].as_str().unwrap();
                assert_eq!(source.text.get(offset..end), Some(fragment));
                rebuilt.push_str(fragment);
                offset = end;
                parts += 1;
                sum_stdout_bytes += raw.stdout.len();
                max_stdout_bytes = max_stdout_bytes.max(raw.stdout.len());
                if retain_sample {
                    let sample = (fs::read(&request_path).unwrap(), raw.stdout.clone());
                    match samples.len() {
                        0 => samples.push(sample),
                        1 => samples.push(sample),
                        _ => samples[1] = sample,
                    }
                }
                let source_metadata = response["source"].clone();
                let source_record_digest = response["recordDigest"].clone();
                assert_eq!(response["source"]["textDigest"], source.text_digest);
                assert_eq!(
                    response["totalTextBytes"].as_u64().unwrap() as usize,
                    source.text.len()
                );

                for line in String::from_utf8_lossy(&raw.stderr).lines() {
                    let Ok(event) = serde_json::from_str::<Value>(line) else {
                        continue;
                    };
                    if event["event"] != "STARTED" {
                        continue;
                    }
                    match event["phase"].as_str().unwrap_or("") {
                        "LOAD_WORK_RECORD" => load_work_started += 1,
                        "LOAD_RETAINED_SNAPSHOT" => load_snapshot_started += 1,
                        phase if acquisition_phases.contains(&phase) => {
                            *acquisition_counts.entry(phase.to_owned()).or_default() += 1;
                        }
                        _ => {}
                    }
                }
                cursor = response["nextCursor"].as_str().map(str::to_owned);
                if cursor.is_none() {
                    let mut reconstructed_metadata = source_metadata;
                    reconstructed_metadata["text"] = json!(rebuilt);
                    let reconstructed_source: Source =
                        serde_json::from_value(reconstructed_metadata).unwrap();
                    let digest = clew::canonical::hash(&reconstructed_source).unwrap();
                    assert_eq!(digest, expected_record_digest);
                    assert_eq!(digest, source_record_digest);
                    assert_eq!(reconstructed_source, source);
                    assert_eq!(offset, source.text.len());
                    assert_eq!(rebuilt, source.text);
                    break digest;
                }
            };

            assert!(load_work_started > 0);
            assert!(load_snapshot_started > 0);
            let acquisition_count_total: usize = acquisition_counts.values().sum();
            assert_eq!(acquisition_count_total, 0, "{acquisition_counts:?}");
            let trap_attempts = fs::read_to_string(&marker)
                .map(|lines| lines.lines().count())
                .unwrap_or(0);
            assert_eq!(trap_attempts, 0, "an acquisition executable was invoked");
            let ledger_after = fs::read(&ledger_path).unwrap();
            let state = work::read_state(&repo, &work_id).unwrap();
            assert_eq!(state.source_part_receipts.len(), parts);
            let ledger_growth = ledger_after.len().saturating_sub(ledger_before);
            assert!(ledger_growth > 0);
            if source.text.is_empty() {
                assert_eq!(parts, 1);
            } else {
                assert!(parts > 0);
            }
            let jobs_after = job_report_count(&jobs_dir);
            let attempts_after = author_attempt_count(&jobs_dir);
            assert_eq!(jobs_after, jobs_before, "read-part must not create jobs");
            assert_eq!(attempts_after, attempts_before);
            let model_invocations = attempts_after.saturating_sub(attempts_before);

            let row = json!({
                "fixture":case_name,
                "maxBytes":max_bytes,
                "sourceTextBytes":source.text.len(),
                "canonicalSourceBytes":canonical_source_bytes,
                "expectedRecordDigest":expected_record_digest,
                "reconstructedCanonicalRecordDigest":reconstructed_record_digest,
                "reconstructedRecordDigestMatches":true,
                "parts":parts,
                "sumStdoutBytesIncludingLf":sum_stdout_bytes,
                "maxStdoutBytesIncludingLf":max_stdout_bytes,
                "ledgerGrowthBytes":ledger_growth,
                "loadWorkRecordStarted":load_work_started,
                "loadRetainedSnapshotStarted":load_snapshot_started,
                "acquisitionStarted":acquisition_counts,
                "acquisitionStartedTotal":acquisition_count_total,
                "executableTrapAttempts":trap_attempts,
                "modelInvocationCount":model_invocations,
                "modelInvocationCountEvidence":"inferred from unchanged persisted author job/attempt counts; read-part accepts no provider configuration",
                "persistedJobReportsBefore":jobs_before,
                "persistedJobReportsAfter":jobs_after,
                "persistedAuthorAttemptsBefore":attempts_before,
                "persistedAuthorAttemptsAfter":attempts_after,
                "finalNextCursorWasNull":true
            });
            results.push(row);

            if retain_sample {
                let stem = format!("samples/{case_name}-{max_bytes}");
                write_optional_artifact(
                    &format!("{stem}-first-request.json"),
                    &samples.first().unwrap().0,
                );
                write_optional_artifact(
                    &format!("{stem}-first-response.json"),
                    &samples.first().unwrap().1,
                );
                write_optional_artifact(
                    &format!("{stem}-last-request.json"),
                    &samples.last().unwrap().0,
                );
                write_optional_artifact(
                    &format!("{stem}-last-response.json"),
                    &samples.last().unwrap().1,
                );
            }
        }
    }

    if std::env::var_os("CODECLEW_SOURCE_PART_ARTIFACT_DIR").is_some() {
        let measurements = json!({
            "schema":"codeclew-source-part-measurements/1.0",
            "method":"raw CLI stdout from docs work read-part, including each response's final LF",
            "cases":[
                {"fixture":"ascii-8k","plannedTextBytes":8192,"budgets":[2048,40960,49152]},
                {"fixture":"mixed-unicode-escaping-12k","plannedTextBytes":12288,"budgets":[2048,40960,49152]},
                {"fixture":"long-line-64k","plannedTextBytes":65536,"containsNewline":false,"budgets":[2048,40960,49152]},
                {"fixture":"empty","plannedTextBytes":0,"budgets":[2048,40960,49152]},
                {"fixture":"small-32b","plannedTextBytes":32,"budgets":[2048,40960,49152]},
                {"fixture":"ascii-1m","plannedTextBytes":1048576,"budgets":[49152]}
            ],
            "rows":results,
            "limitations":{
                "eachCliCallHydratesTheFullRetainedWorkCheck":true,
                "modelInvocationCountEvidence":"derived from zero newly persisted author attempts; the read-part operation has no provider configuration",
                "performanceBaseline":false,
                "testsWriteArtifactsOnlyWhenCodeclewSourcePartArtifactDirIsSet":true
            }
        });
        write_optional_json_artifact("results.json", &measurements);
        write_optional_artifact(
            "README.md",
            b"# Source part read measurements\n\nThis synthetic CLI matrix covers ASCII (8 KiB), mixed Unicode and JSON-escaping text (~12 KiB), one 64 KiB line without a newline, empty text, 32-byte text, and a 1 MiB ASCII source. The first five cases run at Work budgets 2,048, 40,960, and 49,152 bytes; the 1 MiB case runs at 49,152 bytes. Every part was read through the public `docs work read-part` CLI and each response was measured from raw stdout including its final LF. `results.json` records canonical full-SOURCE bytes/digests, text bytes, stdout sums/maxima, ledger growth, retained-load and acquisition spans, trap attempts, and exact canonical record reconstruction. `modelInvocationCount` is inferred from unchanged persisted author job and attempt counts; the read-part command accepts no provider configuration. `proposal-flow.json` and `source-author-guard.json` preserve proposal-completion and zero-dispatch evidence; `non-source-failure-publication.json` records the unchanged non-SOURCE failure path.\n\nReproduce and refresh this opt-in artifact set from the repository root with:\n\n```sh\nCODECLEW_SOURCE_PART_ARTIFACT_DIR=\"$PWD/docs/product/validation/l1-source-parts-20260926\" cargo test --locked -p clew --test docs_work_source_parts -- --test-threads=1\n```\n\nThe CLI continues to hydrate each immutable Work Check in full. This matrix makes no claim about source-memory use, end-to-end author context, or performance. Tests do not write these checked-in samples unless `CODECLEW_SOURCE_PART_ARTIFACT_DIR` is set.\n",
        );
    }
}

#[test]
fn proposal_accepts_only_complete_parts_after_default_pages_and_keeps_reads_sticky() {
    let f = Fixture::new();
    f.service("orders");
    let checked = f.checked();
    let entrypoint_id = checked.services["orders"]
        .entrypoints
        .iter()
        .find(|entrypoint| entrypoint.symbol.contains("reserve"))
        .unwrap()
        .id
        .clone();
    let (checked, snapshot, _source, source_id) = add_synthetic_source(&f, checked, 180 * 1024);
    let (work_id, source_reference) = prepare_work_for_source(
        &f,
        40_960,
        Some(&entrypoint_id),
        Some(&snapshot),
        Some(&source_id),
    );
    let frozen = read(f.docs.join(format!(".codeclew/work/{work_id}/work.json")));
    let entrypoint_reference = frozen["handles"]
        .as_object()
        .unwrap()
        .iter()
        .find(|(_, handle)| handle["kind"] == "ENTRYPOINT" && handle["id"] == entrypoint_id)
        .map(|(reference, _)| reference.clone())
        .expect("selected operation entrypoint handle");
    let selected_entrypoint = checked.services["orders"]
        .entrypoints
        .iter()
        .find(|entrypoint| entrypoint.id == entrypoint_id)
        .unwrap();
    let flow_references: Vec<_> = checked.services["orders"]
        .observations
        .values()
        .filter(|observation| {
            observation.kind == "FLOW" && observation.symbol == selected_entrypoint.symbol
        })
        .map(|observation| {
            frozen["handles"]
                .as_object()
                .unwrap()
                .iter()
                .find(|(_, handle)| {
                    handle["kind"] == "DEPENDENCY" && handle["id"] == observation.id
                })
                .map(|(reference, _)| reference.clone())
                .expect("flow observation dependency handle")
        })
        .collect();
    assert!(
        !flow_references.is_empty(),
        "fixture has entrypoint FLOW evidence"
    );
    let steps: Vec<_> = flow_references
        .iter()
        .map(|reference| {
            json!({
                "kind":"note",
                "meaning":{
                    "text":"The selected operation reaches this captured flow event.",
                    "evidence":[reference]
                }
            })
        })
        .collect();

    let pages = read_default_page_chain(&f, &work_id);
    assert!(!pages.is_empty());
    let all_omissions: Vec<_> = pages
        .iter()
        .flat_map(|page| page["omitted"].as_array().unwrap())
        .collect();
    assert_eq!(
        all_omissions.len(),
        1,
        "only the synthetic SOURCE should be omitted"
    );
    let source_omission = pages
        .iter()
        .flat_map(|page| page["omitted"].as_array().unwrap())
        .find(|row| row["reference"] == source_reference)
        .expect("oversized source must be omitted from the initial page chain");
    assert_eq!(source_omission["kind"], "SOURCE");
    assert_eq!(source_omission["id"], source_id);
    let repo = Repository::open(&f.docs).unwrap();
    let initial_state = work::read_state(&repo, &work_id).unwrap();
    assert!(initial_state.receipts.values().any(|receipt| {
        receipt
            .omitted
            .iter()
            .any(|row| row["reference"] == source_reference)
    }));
    assert!(initial_state.receipts.values().all(|receipt| {
        !receipt
            .supplied
            .iter()
            .any(|reference| reference == &source_reference)
    }));
    assert!(initial_state.source_part_receipts.is_empty());
    let original_omitted: BTreeMap<_, _> = initial_state
        .receipts
        .iter()
        .map(|(key, receipt)| {
            (
                key.clone(),
                clew::canonical::bytes(&receipt.omitted).unwrap(),
            )
        })
        .collect();

    let proposal = json!({
        "schema":"codeclew-documentation-proposal/1.0",
        "operations":[{
            "entrypoint":entrypoint_reference,
            "title":"Quantity entities",
            "summary":{"text":"The retained source defines quantity handling.","evidence":[source_reference]},
            "steps":steps
        }]
    });
    let (_, before_parts) =
        submit_proposal(&f, &work_id, &proposal, "source-part-proposal-before.json");
    assert_eq!(before_parts["status"], "NEEDS_REPAIR", "{before_parts}");
    assert!(
        has_diagnostic(&before_parts, "REQUIRED_CONTEXT_NOT_READ"),
        "{before_parts}"
    );

    let source_text_len = checked.services["orders"].sources[&source_id].text.len();
    let first_part =
        work::read_part(&repo, &work_id, request_value(&source_reference, None)).unwrap();
    assert_eq!(first_part["startByte"], 0);
    assert!(first_part["nextCursor"].is_string());
    let (_, after_first_part) = submit_proposal(
        &f,
        &work_id,
        &proposal,
        "source-part-proposal-after-first-part.json",
    );
    assert_eq!(
        after_first_part["status"], "NEEDS_REPAIR",
        "{after_first_part}"
    );
    assert!(has_diagnostic(
        &after_first_part,
        "REQUIRED_CONTEXT_NOT_READ"
    ));

    // A proposal submitted before a new part receipt carries the earlier
    // optimistic read digest and cannot be published after the receipt lands.
    let (stale_code, stale_publish) = f.run(&[
        "docs",
        "proposal",
        "publish",
        "--proposal",
        before_parts["proposal"].as_str().unwrap(),
        "--unassessed",
    ]);
    assert_ne!(stale_code, 0, "{stale_publish}");
    assert!(
        stale_publish.to_string().contains("work reads changed"),
        "{stale_publish}"
    );

    let mut cursor = first_part["nextCursor"].as_str().map(str::to_owned);
    let mut next_offset = 0usize;
    let mut part_count = 1usize;
    let mut rebuilt = String::with_capacity(source_text_len);
    let first_fragment = first_part["text"].as_str().unwrap();
    assert_eq!(
        first_part["startByte"].as_u64().unwrap() as usize,
        next_offset
    );
    next_offset = first_part["endByte"].as_u64().unwrap() as usize;
    rebuilt.push_str(first_fragment);
    loop {
        let Some(current_cursor) = cursor.as_deref() else {
            break;
        };
        let part = work::read_part(
            &repo,
            &work_id,
            request_value(&source_reference, Some(current_cursor)),
        )
        .unwrap();
        part_count += 1;
        assert_eq!(part["startByte"].as_u64().unwrap() as usize, next_offset);
        let fragment = part["text"].as_str().unwrap();
        next_offset = part["endByte"].as_u64().unwrap() as usize;
        rebuilt.push_str(fragment);
        cursor = part["nextCursor"].as_str().map(str::to_owned);
        if cursor.is_none() {
            break;
        }
    }
    assert_eq!(rebuilt, checked.services["orders"].sources[&source_id].text);
    let complete_state = work::read_state(&repo, &work_id).unwrap();
    let mut ordered_parts: Vec<_> = complete_state
        .source_part_receipts
        .values()
        .cloned()
        .collect();
    ordered_parts.sort_by_key(|receipt| receipt.start_byte);
    assert!(ordered_parts.len() >= 3);
    let reads_path = f.docs.join(format!(".codeclew/work/{work_id}/reads.json"));
    let complete_ledger_bytes = fs::read(&reads_path).unwrap();

    let mut last_only = complete_state.clone();
    last_only.source_part_receipts.clear();
    let last = ordered_parts.last().unwrap().clone();
    last_only
        .source_part_receipts
        .insert(last.receipt_digest.clone(), last);
    fs::write(&reads_path, clew::canonical::bytes(&last_only).unwrap()).unwrap();
    let (_, last_only_result) = submit_proposal(
        &f,
        &work_id,
        &proposal,
        "source-part-proposal-last-only.json",
    );
    assert_eq!(
        last_only_result["status"], "NEEDS_REPAIR",
        "{last_only_result}"
    );
    assert!(has_diagnostic(
        &last_only_result,
        "REQUIRED_CONTEXT_NOT_READ"
    ));

    let middle_index = ordered_parts.len() / 2;
    let mut missing_middle = complete_state.clone();
    missing_middle.source_part_receipts.clear();
    for (index, receipt) in ordered_parts.iter().enumerate() {
        if index != middle_index {
            missing_middle
                .source_part_receipts
                .insert(receipt.receipt_digest.clone(), receipt.clone());
        }
    }
    assert!(
        missing_middle
            .source_part_receipts
            .contains_key(&ordered_parts.first().unwrap().receipt_digest)
    );
    assert!(
        missing_middle
            .source_part_receipts
            .contains_key(&ordered_parts.last().unwrap().receipt_digest)
    );
    fs::write(
        &reads_path,
        clew::canonical::bytes(&missing_middle).unwrap(),
    )
    .unwrap();
    let (_, missing_middle_result) = submit_proposal(
        &f,
        &work_id,
        &proposal,
        "source-part-proposal-missing-middle.json",
    );
    assert_eq!(
        missing_middle_result["status"], "NEEDS_REPAIR",
        "{missing_middle_result}"
    );
    assert!(has_diagnostic(
        &missing_middle_result,
        "REQUIRED_CONTEXT_NOT_READ"
    ));
    fs::write(&reads_path, &complete_ledger_bytes).unwrap();

    let after_parts = work::read_state(&repo, &work_id).unwrap();
    let retained_omitted: BTreeMap<_, _> = after_parts
        .receipts
        .iter()
        .map(|(key, receipt)| {
            (
                key.clone(),
                clew::canonical::bytes(&receipt.omitted).unwrap(),
            )
        })
        .collect();
    assert_eq!(retained_omitted, original_omitted);

    let (code, after_parts_result) =
        submit_proposal(&f, &work_id, &proposal, "source-part-proposal-after.json");
    assert_eq!(code, 0, "{after_parts_result}");
    assert!(
        matches!(
            after_parts_result["status"].as_str(),
            Some("READY_FOR_REVIEW" | "READY_WITH_LIMITATIONS")
        ),
        "{after_parts_result}"
    );
    assert!(!has_diagnostic(
        &after_parts_result,
        "REQUIRED_CONTEXT_NOT_READ"
    ));
    assert!(!has_diagnostic(&after_parts_result, "INVALID_PROPOSAL"));
    write_optional_json_artifact(
        "proposal-flow.json",
        &json!({
            "schema":"codeclew-source-part-proposal-evidence/1.0",
            "initialProposalStatus":before_parts["status"],
            "initialDiagnosticCodes":diagnostic_codes(&before_parts),
            "afterFirstPartProposalStatus":after_first_part["status"],
            "afterFirstPartDiagnosticCodes":diagnostic_codes(&after_first_part),
            "firstPartEndByte":first_part["endByte"],
            "stalePrePartProposalRejectedAfterFirstReceipt":true,
            "stalePublishError":stale_publish.to_string(),
            "completeProposalStatus":after_parts_result["status"],
            "completeProposalDiagnosticCodes":diagnostic_codes(&after_parts_result),
            "manualPartCount":part_count,
            "sourceTextBytes":source_text_len,
            "sourceRecordDigest":first_part["recordDigest"],
            "sourcePartReceiptCount":after_parts.source_part_receipts.len(),
            "ordinaryDefaultPagesOmittedSource":true,
            "ordinaryDefaultPagesSuppliedSource":false,
            "historicalOmittedArraysByteIdentical":retained_omitted == original_omitted
        }),
    );

    // Any untracked read remains sticky even after every SOURCE byte is recorded.
    let untracked_input = f.input(
        "source-part-untracked-selection.json",
        &json!({"references":[entrypoint_reference],"untrackedReads":true}),
    );
    let (code, untracked_page) = f.run(&[
        "docs",
        "work",
        "read",
        "--work",
        &work_id,
        "--input",
        untracked_input.to_str().unwrap(),
    ]);
    assert_eq!(code, 0, "{untracked_page}");
    let (_, untracked_proposal) = submit_proposal(
        &f,
        &work_id,
        &proposal,
        "source-part-proposal-untracked.json",
    );
    assert_eq!(untracked_proposal["status"], "NEEDS_REPAIR");
    assert!(has_diagnostic(&untracked_proposal, "INCOMPLETE_INFLUENCE"));

    let declaration_path = f.docs.join("catalog/services/orders.json");
    let mut declaration = read(&declaration_path);
    declaration["title"] = json!("Changed after retained Work");
    fs::write(&declaration_path, serde_json::to_vec(&declaration).unwrap()).unwrap();
    let input = f.input("source-part-proposal-stale.json", &proposal);
    let output = f.run_raw(&[
        "docs",
        "proposal",
        "submit",
        "--work",
        &work_id,
        "--input",
        input.to_str().unwrap(),
    ]);
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("stale")
            || String::from_utf8_lossy(&output.stdout).contains("STALE"),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
}

#[cfg(target_os = "macos")]
#[test]
fn manual_source_parts_do_not_dispatch_or_publish_an_oversized_author_request() {
    let f = Fixture::new();
    let source_repo = f.service("orders");
    let checked = f.checked();
    let entrypoint_id = checked.services["orders"]
        .entrypoints
        .iter()
        .find(|entrypoint| entrypoint.symbol.contains("reserve"))
        .unwrap()
        .id
        .clone();
    let (_checked, snapshot, _source, source_id) = add_synthetic_source(&f, checked, 180 * 1024);
    let (work_id, source_reference) = prepare_work_for_source(
        &f,
        40_960,
        Some(&entrypoint_id),
        Some(&snapshot),
        Some(&source_id),
    );
    let pages = read_default_page_chain(&f, &work_id);
    assert!(
        pages
            .iter()
            .flat_map(|page| page["omitted"].as_array().unwrap())
            .any(|row| { row["kind"] == "SOURCE" && row["reference"] == source_reference })
    );

    let repo = Repository::open(&f.docs).unwrap();
    let mut cursor = None;
    let mut parts = 0usize;
    loop {
        let response = work::read_part(
            &repo,
            &work_id,
            request_value(&source_reference, cursor.as_deref()),
        )
        .unwrap();
        parts += 1;
        cursor = response["nextCursor"].as_str().map(str::to_owned);
        if cursor.is_none() {
            break;
        }
    }
    assert!(parts >= 3);

    let _rendered = f.ok(&["docs", "render", "--snapshot", &snapshot]);
    let index_path = f.docs.join("docs/index.html");
    let index_before = fs::read(&index_path).unwrap();
    let driver = f.temp.path().join("never-dispatch-author.sh");
    let driver_marker = f.temp.path().join("unexpected-author-driver-invocation");
    fs::write(
        &driver,
        format!(
            "#!/bin/sh\nprintf '%s\\n' invoked >> '{}'\nexit 91\n",
            driver_marker.display()
        ),
    )
    .unwrap();
    fs::set_permissions(&driver, fs::Permissions::from_mode(0o700)).unwrap();
    let config = f.input(
        "source-part-execution.json",
        &execution_config(&driver, "source-parts-fixture"),
    );

    fs::rename(&source_repo, source_repo.with_extension("offline")).unwrap();
    let latest_path = f.docs.join(".codeclew/cache/latest-check.json");
    let poisoned_latest = b"source-part author guard must stay pinned\n";
    fs::write(&latest_path, poisoned_latest).unwrap();
    let trap_dir = f.temp.path().join("author-acquisition-traps");
    fs::create_dir(&trap_dir).unwrap();
    let marker = f.temp.path().join("unexpected-author-acquisition");
    for executable in ["git", "java", "javac", "gradle", "mvn", "kotlinc"] {
        let path = trap_dir.join(executable);
        fs::write(
            &path,
            format!(
                "#!/bin/sh\nprintf '%s\\n' '{}' >> '{}'\nexit 91\n",
                executable,
                marker.display()
            ),
        )
        .unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
    }
    let output = f.run_raw_with_path(
        &[
            "docs",
            "work",
            "run",
            "--work",
            &work_id,
            "--config",
            config.to_str().unwrap(),
        ],
        &trap_dir,
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let result: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["status"], "NEEDS_EVIDENCE", "{result}");
    assert!(
        result["gap"]["reason"]
            .as_str()
            .unwrap()
            .contains("INITIAL_SOURCE_EXCEEDS_WORK_BYTE_BUDGET")
    );
    assert!(
        result["gap"]["reason"]
            .as_str()
            .unwrap()
            .contains("Manual docs work read-part")
    );
    assert!(
        result["gap"]["reason"]
            .as_str()
            .unwrap()
            .contains("automatic author request")
    );
    assert!(result["publication"].is_null(), "{result}");
    assert_eq!(fs::read(&index_path).unwrap(), index_before);
    assert_eq!(fs::read(latest_path).unwrap(), poisoned_latest);
    assert!(!marker.exists(), "an acquisition executable was invoked");
    assert!(
        !driver_marker.exists(),
        "the configured author driver was invoked"
    );

    let report = read(f.docs.join(format!(
        ".codeclew/jobs/{}.json",
        result["run"].as_str().unwrap()
    )));
    assert!(
        report["attempts"].as_array().unwrap().is_empty(),
        "{report}"
    );
    assert!(
        report["accounting"].as_array().unwrap().is_empty(),
        "{report}"
    );
    assert!(report["publication"].is_null(), "{report}");
    let account = read(f.docs.join("execution/accounts/source-parts-fixture.json"));
    assert!(account["reservations"].as_object().unwrap().is_empty());

    let stderr = String::from_utf8_lossy(&output.stderr);
    let progress: Vec<Value> = stderr
        .lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect();
    let mut acquisition_started = BTreeMap::from([
        ("ACQUIRE_SERVICE_EVIDENCE".to_owned(), 0usize),
        ("ACQUIRE_SYNTAX_EVIDENCE".to_owned(), 0usize),
        ("ACQUIRE_COMPILER_EVIDENCE".to_owned(), 0usize),
        ("ENSURE_COMPILER_GENERATION".to_owned(), 0usize),
    ]);
    for event in &progress {
        if event["event"] == "STARTED"
            && let Some(count) = event["phase"]
                .as_str()
                .and_then(|phase| acquisition_started.get_mut(phase))
        {
            *count += 1;
        }
    }
    let read_state = work::read_state(&Repository::open(&f.docs).unwrap(), &work_id).unwrap();
    let author_attempt_count = report["attempts"].as_array().unwrap().len();
    let accounting_record_count = report["accounting"].as_array().unwrap().len();
    let reservation_count = account["reservations"].as_object().unwrap().len();
    assert!(progress.iter().any(|event| {
        event["phase"] == "LOAD_RETAINED_SNAPSHOT" && event["event"] == "STARTED"
    }));
    for phase in [
        "ACQUIRE_SERVICE_EVIDENCE",
        "ACQUIRE_SYNTAX_EVIDENCE",
        "ACQUIRE_COMPILER_EVIDENCE",
        "ENSURE_COMPILER_GENERATION",
    ] {
        assert!(
            !progress
                .iter()
                .any(|event| event["phase"] == phase && event["event"] == "STARTED")
        );
    }
    write_optional_json_artifact(
        "source-author-guard.json",
        &json!({
            "schema":"codeclew-source-part-author-guard-evidence/1.0",
            "status":result["status"],
            "diagnosticCode":"INITIAL_SOURCE_EXCEEDS_WORK_BYTE_BUDGET",
            "diagnostic":result["gap"]["reason"],
            "manualPartCount":parts,
            "sourcePartReceiptCount":read_state.source_part_receipts.len(),
            "authorAttemptCount":author_attempt_count,
            "accountingRecordCount":accounting_record_count,
            "reservationCount":reservation_count,
            "resultPublication":result["publication"],
            "jobReportPublication":report["publication"],
            "docsIndexUnchanged":fs::read(&index_path).unwrap() == index_before,
            "latestPointerFollowed":false,
            "sourceRepoOffline":source_repo.with_extension("offline").exists(),
            "acquisitionStarted":acquisition_started,
            "acquisitionStartedTotal":acquisition_started.values().sum::<usize>(),
            "retainedSnapshotLoadStarted":true,
            "acquisitionExecutableTrapAttempts":0,
            "configuredAuthorDriverMarkerAttempts":0,
            "configuredDriverWasInvoked":driver_marker.exists()
        }),
    );
}

#[cfg(target_os = "macos")]
#[test]
fn non_source_oversized_author_context_keeps_legacy_failure_publication() {
    let f = Fixture::new();
    f.service("orders");
    fs::create_dir(f.docs.join("notes")).unwrap();
    fs::write(
        f.docs.join("notes/long.md"),
        "long retained documentation note ".repeat(6_000),
    )
    .unwrap();
    let checked = f.checked();
    let repo = Repository::open(&f.docs).unwrap();
    let snapshot = checked.save_snapshot(&repo).unwrap();
    let (work_id, _) = prepare_work_for(&f, 40_960, None, Some(&snapshot));
    let pages = read_default_page_chain(&f, &work_id);
    let omitted: Vec<_> = pages
        .iter()
        .flat_map(|page| page["omitted"].as_array().unwrap())
        .collect();
    assert!(
        omitted
            .iter()
            .any(|row| { row["kind"] == "EXTERNAL_INPUT" && row["id"] == "notes/long.md" }),
        "{omitted:?}"
    );
    assert!(
        omitted.iter().all(|row| row["kind"] != "SOURCE"),
        "fixture should exercise only the legacy non-SOURCE path: {omitted:?}"
    );

    let _rendered = f.ok(&["docs", "render", "--snapshot", &snapshot]);
    let driver = f.temp.path().join("legacy-non-source-driver.sh");
    fs::write(&driver, "#!/bin/sh\nexit 91\n").unwrap();
    fs::set_permissions(&driver, fs::Permissions::from_mode(0o700)).unwrap();
    let config = f.input(
        "legacy-non-source-execution.json",
        &execution_config(&driver, "legacy-non-source-fixture"),
    );
    let output = f.run_raw(&[
        "docs",
        "work",
        "run",
        "--work",
        &work_id,
        "--config",
        config.to_str().unwrap(),
    ]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let result: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["status"], "NEEDS_EVIDENCE", "{result}");
    assert_eq!(
        result["gap"]["reason"],
        "NEEDS_EVIDENCE: a required initial record exceeds the work budget",
        "{result}"
    );
    assert!(result["publication"].is_object(), "{result}");
    let report = read(f.docs.join(format!(
        ".codeclew/jobs/{}.json",
        result["run"].as_str().unwrap()
    )));
    assert!(
        report["attempts"].as_array().unwrap().is_empty(),
        "{report}"
    );
    assert!(
        report["accounting"].as_array().unwrap().is_empty(),
        "{report}"
    );
    assert!(report["publication"].is_object(), "{report}");
    let account = read(
        f.docs
            .join("execution/accounts/legacy-non-source-fixture.json"),
    );
    assert!(account["reservations"].as_object().unwrap().is_empty());
    write_optional_json_artifact(
        "non-source-failure-publication.json",
        &json!({
            "schema":"codeclew-source-part-legacy-failure-evidence/1.0",
            "omittedKind":"EXTERNAL_INPUT",
            "diagnostic":result["gap"]["reason"],
            "resultPublicationPresent":result["publication"].is_object(),
            "jobReportPublicationPresent":report["publication"].is_object(),
            "authorAttemptCount":report["attempts"].as_array().unwrap().len(),
            "accountingRecordCount":report["accounting"].as_array().unwrap().len(),
            "reservationCount":account["reservations"].as_object().unwrap().len()
        }),
    );
}
