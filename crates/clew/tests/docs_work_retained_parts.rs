//! Public CLI regressions for bounded historical record reads.
#![cfg(unix)]

#[path = "support/documentation.rs"]
mod support;

use clew::{
    canonical,
    documentation::{check::Check, model::Operation, store::Repository, work},
};
use serde_json::{Value, json};
use std::{collections::BTreeMap, fs, path::Path, process::Output};
use support::{Fixture, read};

const MAX_BYTES: usize = 49_152;
const REQUEST_SCHEMA: &str = "codeclew-documentation-retained-part-request/1.0";

fn bundle_files(root: &Path) -> BTreeMap<String, Vec<u8>> {
    fn visit(root: &Path, directory: &Path, result: &mut BTreeMap<String, Vec<u8>>) {
        for entry in fs::read_dir(directory).unwrap() {
            let entry = entry.unwrap();
            if entry.file_type().unwrap().is_dir() {
                visit(root, &entry.path(), result);
            } else {
                assert!(entry.file_type().unwrap().is_file());
                result.insert(
                    entry
                        .path()
                        .strip_prefix(root)
                        .unwrap()
                        .to_str()
                        .unwrap()
                        .into(),
                    fs::read(entry.path()).unwrap(),
                );
            }
        }
    }
    let mut result = BTreeMap::new();
    visit(root, root, &mut result);
    result
}

fn successful_json(output: Output) -> Value {
    assert!(
        output.status.success(),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    assert_eq!(output.stdout.last(), Some(&b'\n'));
    assert!(
        output.stdout.len() <= MAX_BYTES,
        "{} response bytes",
        output.stdout.len()
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

fn prepare(f: &Fixture, snapshot: &str, entrypoint: &str, audience: &str) -> String {
    let input = f.input(
        "retained-work-request.json",
        &json!({
            "schema":"codeclew-documentation-work-request/1.0", "audience":audience,
            "entrypoint":entrypoint, "maxItems":100, "maxBytes":MAX_BYTES,
        }),
    );
    f.ok(&[
        "docs",
        "work",
        "prepare",
        "--subject",
        "service:orders",
        "--snapshot",
        snapshot,
        "--input",
        input.to_str().unwrap(),
    ])["work"]
        .as_str()
        .unwrap()
        .to_owned()
}

fn read_pages(f: &Fixture, id: &str) -> Vec<Value> {
    read_selected_pages(f, id, json!({}))
}

fn read_selected_pages(f: &Fixture, id: &str, selection: Value) -> Vec<Value> {
    let mut cursor = None;
    let mut pages = Vec::new();
    loop {
        let mut selection = selection.clone();
        if let Some(cursor) = &cursor {
            selection["cursor"] = json!(cursor);
        }
        let input = f.input("retained-work-selection.json", &selection);
        let page = successful_json(f.run_raw(&[
            "docs",
            "work",
            "read",
            "--work",
            id,
            "--input",
            input.to_str().unwrap(),
        ]));
        let next = page["nextCursor"].as_str().map(str::to_owned);
        assert!(
            next.is_none() || next != cursor,
            "page cursor must progress"
        );
        cursor = next;
        pages.push(page);
        if cursor.is_none() {
            return pages;
        }
    }
}

fn part(f: &Fixture, work: &str, id: &str, kind: &str, cursor: Option<&str>) -> Output {
    let mut request = json!({"schema":REQUEST_SCHEMA, "kind":kind, "id":id});
    if let Some(cursor) = cursor {
        request["cursor"] = json!(cursor);
    }
    let input = f.input("retained-part-request.json", &request);
    f.run_raw(&[
        "docs",
        "work",
        "read-retained-part",
        "--work",
        work,
        "--input",
        input.to_str().unwrap(),
    ])
}

#[test]
fn oversized_openapi_dependency_is_readable_through_the_public_cli() {
    let f = Fixture::new();
    let source = f.temp.path().join("orders");
    fs::create_dir(&source).unwrap();
    fs::write(
        source.join("Orders.java"),
        "class Orders { String accept(String request) { return request; } }",
    )
    .unwrap();
    // Reproduce the reference-sibling boundary that made the real 3.1.2
    // contract-operation record exceed the Work response budget.
    let component = "LongReference".repeat(8000);
    let reference = format!("#/components/schemas/{component}");
    let contract = json!({"openapi":"3.1.2","info":{"title":"Orders","version":"1"},
        "paths":{"/accept":{"post":{"operationId":"accept","requestBody":{"content":{"application/json":{"schema":{"$ref":reference,"description":"Reference sibling"}}}},"responses":{"200":{"description":"Accepted"}}}}},
        "components":{"schemas":{"Small":{"type":"object","properties":{"id":{"type":"string"}}}}}});
    fs::write(
        source.join("api.json"),
        serde_json::to_vec(&contract).unwrap(),
    )
    .unwrap();
    support::git(&source, &["init", "-q"]);
    support::git(
        &source,
        &["remote", "add", "origin", "https://example.invalid/orders"],
    );
    support::commit(&source);
    let record = f.input("dependency-service.json",&json!({"schema":"codeclew-documentation-service/1.0","id":"orders","title":"Orders","repositoryId":"orders","repository":"https://example.invalid/orders","language":"java","profile":"source-syntax","targetRef":"HEAD","source":{"roots":["Orders.java"],"dialect":"17"},"contractFiles":["api.json"]}));
    let input_digest = f.ok(&["docs", "service", "list"])["inputDigest"]
        .as_str()
        .unwrap()
        .to_owned();
    f.ok(&[
        "docs",
        "service",
        "add",
        "--input",
        record.to_str().unwrap(),
        "--expected-input-digest",
        &input_digest,
    ]);
    f.ok(&[
        "docs",
        "bind",
        "--service",
        "orders",
        "--repo",
        source.to_str().unwrap(),
    ]);
    let (code, checked_result) = f.run(&["docs", "check"]);
    assert!(matches!(code, 0 | 3 | 4), "{checked_result}");
    let snapshot = checked_result["snapshot"].as_str().unwrap();
    let checked = Check::load_snapshot(&Repository::open(&f.docs).unwrap(), snapshot).unwrap();
    let request=f.input("dependency-work.json",&json!({"schema":"codeclew-documentation-work-request/1.0","audience":"API integrator","maxItems":100,"maxBytes":MAX_BYTES}));
    let prepared = f.ok(&[
        "docs",
        "work",
        "prepare",
        "--subject",
        "service:orders",
        "--snapshot",
        snapshot,
        "--input",
        request.to_str().unwrap(),
    ]);
    let work_id = prepared["work"].as_str().unwrap();
    fs::rename(&source, source.with_extension("offline")).unwrap();
    let pages = read_selected_pages(
        &f,
        work_id,
        json!({"query":{"kind":"CONTRACT_OPERATION","symbolContains":""}}),
    );
    let omitted: Vec<_> = pages
        .iter()
        .flat_map(|p| p["omitted"].as_array().unwrap())
        .filter(|r| r["kind"] == "DEPENDENCY")
        .collect();
    assert!(!omitted.is_empty(), "{pages:?}");
    for row in omitted {
        assert_eq!(row["reason"], "ITEM_EXCEEDS_WORK_BYTE_BUDGET");
        assert_eq!(row["recovery"]["command"], "docs work read-dependency-part");
        let id = row["id"].as_str().unwrap();
        let mut cursor = None;
        let mut text = String::new();
        loop {
            let request=f.input("dependency-part.json",&json!({"schema":"codeclew-documentation-dependency-part-request/1.0","kind":"DEPENDENCY","id":id,"cursor":cursor}));
            let part = successful_json(f.run_raw(&[
                "docs",
                "work",
                "read-dependency-part",
                "--work",
                work_id,
                "--input",
                request.to_str().unwrap(),
            ]));
            assert_eq!(part["startByte"].as_u64().unwrap() as usize, text.len());
            text.push_str(part["text"].as_str().unwrap());
            assert_eq!(part["endByte"].as_u64().unwrap() as usize, text.len());
            cursor = part["nextCursor"].as_str().map(str::to_owned);
            if cursor.is_none() {
                break;
            }
        }
        let expected = serde_json::to_value(&checked.dependencies[id]).unwrap();
        assert_eq!(serde_json::from_str::<Value>(&text).unwrap(), expected);
    }
}

#[test]
fn oversized_historical_operation_is_part_readable_offline() {
    let f = Fixture::new();
    // This public source-syntax fixture uses the native parser, without a JVM or
    // compiler worker. The oversized record below is authored prose, not source.
    let source_repo = f.service("orders");
    let (code, checked_result) = f.run(&["docs", "check"]);
    assert!(matches!(code, 0 | 3 | 4), "{checked_result}");
    let snapshot = checked_result["snapshot"].as_str().unwrap().to_owned();
    let repo = Repository::open(&f.docs).unwrap();
    let checked = Check::load_snapshot(&repo, &snapshot).unwrap();
    let narrative_path = f.author("orders", &checked);
    let mut narrative = read(&narrative_path);
    let entrypoint = narrative["operations"][0]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let paragraph_template = narrative["operations"][0]["explanation"][0].clone();
    let prose = "Synthetic retained explanation λ🔥: the quantity is normalized. Quote \" and slash \\ remain literal.\n".repeat(60);
    assert!(prose.len() <= 8192);
    for index in 0..12 {
        let mut paragraph = paragraph_template.clone();
        paragraph["id"] = json!(format!("retained-paragraph-{index}"));
        paragraph["text"] = json!(format!(
            "Retained fixture paragraph {index}. {prose} END_終端_{index}"
        ));
        assert!(paragraph["text"].as_str().unwrap().len() <= 8192);
        paragraph["detail"] = json!(index % 2 == 0);
        narrative["operations"][0]["explanation"]
            .as_array_mut()
            .unwrap()
            .push(paragraph);
    }
    let authored: Operation = serde_json::from_value(narrative["operations"][0].clone()).unwrap();
    let expected = canonical::bytes(&authored).unwrap();
    assert!(expected.len() > MAX_BYTES);
    let input = f.input("oversized-authored-narrative.json", &narrative);
    // Fixed historical storage record, not a replacement document generator.
    let bundle = f.retain_legacy(&checked, &input);
    let baseline = f.bindings(&bundle);
    let retained = baseline["narratives"]["service:orders"]["operations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|operation| operation["id"] == entrypoint)
        .unwrap();
    let original: Operation = serde_json::from_value(retained.clone()).unwrap();
    assert_eq!(canonical::bytes(&original).unwrap(), expected);
    let frozen_bundle = bundle_files(&f.bundle(&bundle, ""));
    let index_before = fs::read(f.docs.join("docs/index.html")).unwrap();
    let latest_before = fs::read(f.docs.join(".codeclew/cache/latest-check.json")).unwrap();

    let work_id = prepare(
        &f,
        &snapshot,
        &entrypoint,
        "Synthetic retained title correction",
    );
    let work_manifest = f.docs.join(format!(".codeclew/work/{work_id}/work.json"));
    let work_before = fs::read(&work_manifest).unwrap();
    // All subsequent reads, preparations and submission must reuse saved input.
    fs::rename(&source_repo, source_repo.with_extension("offline")).unwrap();
    let pages = read_pages(&f, &work_id);
    let omitted: Vec<_> = pages
        .iter()
        .flat_map(|page| page["omitted"].as_array().unwrap())
        .collect();
    assert_eq!(omitted.len(), 1, "{omitted:?}");
    assert_eq!(omitted[0]["kind"], "RETAINED_OPERATION");
    assert_eq!(omitted[0]["id"], entrypoint);
    assert_eq!(omitted[0]["reason"], "ITEM_EXCEEDS_WORK_BYTE_BUDGET");
    assert!(omitted[0]["reference"].is_null());

    let first = successful_json(part(&f, &work_id, &entrypoint, "RETAINED_OPERATION", None));
    assert_eq!(first["work"], work_id);
    assert_eq!(first["snapshot"], snapshot);
    assert_eq!(first["recordDigest"], canonical::hash(&original).unwrap());
    assert_eq!(
        first["totalRecordBytes"].as_u64().unwrap() as usize,
        expected.len()
    );
    let first_cursor = first["nextCursor"]
        .as_str()
        .expect("oversized record requires continuation")
        .to_owned();
    let reads_path = f.docs.join(format!(".codeclew/work/{work_id}/reads.json"));
    let interrupted_reads = fs::read(&reads_path).unwrap();
    // Repeat after a separate process invocation: it cannot replace missing parts.
    assert_eq!(
        successful_json(part(&f, &work_id, &entrypoint, "RETAINED_OPERATION", None)),
        first
    );
    assert_eq!(fs::read(&reads_path).unwrap(), interrupted_reads);
    let mut text = first["text"].as_str().unwrap().to_owned();
    let mut cursor = Some(first_cursor.clone());
    while let Some(saved_cursor) = cursor {
        let response = successful_json(part(
            &f,
            &work_id,
            &entrypoint,
            "RETAINED_OPERATION",
            Some(&saved_cursor),
        ));
        assert_eq!(response["startByte"].as_u64().unwrap() as usize, text.len());
        assert_eq!(response["recordDigest"], first["recordDigest"]);
        assert_eq!(response["work"], work_id);
        assert_eq!(response["snapshot"], snapshot);
        let fragment = response["text"].as_str().unwrap();
        assert!(!fragment.is_empty());
        assert_eq!(
            response["fragmentDigest"],
            canonical::hash_bytes(fragment.as_bytes())
        );
        text.push_str(fragment);
        assert_eq!(response["endByte"].as_u64().unwrap() as usize, text.len());
        let next = response["nextCursor"].as_str().map(str::to_owned);
        assert!(next.as_deref() != Some(saved_cursor.as_str()));
        cursor = next;
    }
    assert_eq!(text.as_bytes(), expected);
    assert_eq!(serde_json::from_str::<Operation>(&text).unwrap(), original);
    let other_work = prepare(
        &f,
        &snapshot,
        &entrypoint,
        "Another immutable Work with the same retained operation",
    );
    assert_ne!(other_work, work_id);
    let other_manifest = f
        .docs
        .join(format!(".codeclew/work/{other_work}/work.json"));
    let other_before = fs::read(&other_manifest).unwrap();
    let other_reads = f
        .docs
        .join(format!(".codeclew/work/{other_work}/reads.json"));
    let other_reads_before = fs::read(&other_reads).unwrap();
    let reads_before_rejection = fs::read(&reads_path).unwrap();
    for (rejected, expected_error) in [
        (
            part(&f, &work_id, &entrypoint, "DEPENDENCY", None),
            "retained-part request requires",
        ),
        (
            part(
                &f,
                &work_id,
                "unknown-operation",
                "RETAINED_OPERATION",
                None,
            ),
            "retained operation is outside the Work's selected entrypoint",
        ),
        (
            part(
                &f,
                &other_work,
                &entrypoint,
                "RETAINED_OPERATION",
                Some(&first_cursor),
            ),
            "retained-part cursor belongs to another Work",
        ),
    ] {
        assert!(
            !rejected.status.success(),
            "{}",
            String::from_utf8_lossy(&rejected.stdout)
        );
        let error: Value = serde_json::from_slice(&rejected.stdout).unwrap();
        assert!(error.to_string().contains(expected_error), "{error}");
    }
    assert_eq!(fs::read(&reads_path).unwrap(), reads_before_rejection);
    assert_eq!(fs::read(&other_reads).unwrap(), other_reads_before);
    assert_eq!(fs::read(&work_manifest).unwrap(), work_before);
    assert_eq!(fs::read(&other_manifest).unwrap(), other_before);
    assert_eq!(
        fs::read(f.docs.join("docs/index.html")).unwrap(),
        index_before
    );
    assert_eq!(
        fs::read(f.docs.join(".codeclew/cache/latest-check.json")).unwrap(),
        latest_before
    );
    assert_eq!(bundle_files(&f.bundle(&bundle, "")), frozen_bundle);
    let frozen = work::load(&repo, &work_id).unwrap();
    assert_eq!(
        frozen
            .retained
            .as_ref()
            .unwrap()
            .operations
            .iter()
            .find(|operation| operation.id == entrypoint)
            .unwrap(),
        &original
    );
}
