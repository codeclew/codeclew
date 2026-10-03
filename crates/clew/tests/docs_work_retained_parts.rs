//! Public CLI regression for an oversized authored operation and a bounded edit.
//! Read/proposal readiness is deliberately separate from reviewed publication.
#![cfg(unix)]

#[path = "support/documentation.rs"]
mod support;

use clew::{
    canonical,
    documentation::{check::Check, model::Operation, proposals, store::Repository, work},
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
    let mut cursor = None;
    let mut pages = Vec::new();
    loop {
        let selection = cursor
            .as_ref()
            .map_or_else(|| json!({}), |cursor| json!({"cursor":cursor}));
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

fn submit(f: &Fixture, work: &str, proposal: &Value) -> Value {
    let input = f.input("retained-title-proposal.json", proposal);
    f.ok(&[
        "docs",
        "proposal",
        "submit",
        "--work",
        work,
        "--input",
        input.to_str().unwrap(),
    ])
}

#[test]
fn oversized_authored_operation_is_part_readable_and_title_edit_is_machine_ready_offline() {
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
    // Seed an ordinary frozen publication through the supported narrative path.
    // This is not a claim of separate meaning review for the synthetic fixture.
    let publication = f.ok(&[
        "docs",
        "render",
        "--snapshot",
        &snapshot,
        "--input",
        input.to_str().unwrap(),
        "--publish",
    ]);
    assert_eq!(publication["updateFailures"], json!({}), "{publication}");
    let bundle = publication["bundle"].as_str().unwrap().to_owned();
    let baseline = read(f.bundle(&bundle, "bindings.json"));
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
    assert_eq!(
        f.ok(&["docs", "history", "show", "--id", &bundle])["status"],
        "FROZEN_SNAPSHOT"
    );

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
    let replacement = "Reserve the normalized requested quantity";
    let proposal = json!({"schema":"codeclew-documentation-proposal/1.0", "operations":[],
        "retainedEdits":[{"kind":"RETAINED_OPERATION", "id":entrypoint,
            "recordDigest":first["recordDigest"], "target":"operationTitle",
            "expectedOldValue":original.title, "replacement":replacement}]});
    let partial = submit(&f, &work_id, &proposal);
    assert_eq!(partial["status"], "NEEDS_REPAIR", "{partial}");
    assert!(
        partial["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["kind"] == "DIAGNOSTIC"
                && item["record"]["code"] == "REQUIRED_CONTEXT_NOT_READ")
    );

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
    let ready = submit(&f, &work_id, &proposal);
    assert!(
        ready["status"].as_str().unwrap().starts_with("READY_"),
        "{ready}"
    );
    let artifact = proposals::load(&repo, ready["proposal"].as_str().unwrap()).unwrap();
    assert_eq!(artifact.meaning_review, "UNASSESSED");
    let edited = artifact
        .narrative
        .as_ref()
        .unwrap()
        .operations
        .iter()
        .find(|operation| operation.id == entrypoint)
        .unwrap();
    let mut expected_edit = original.clone();
    expected_edit.title = replacement.into();
    assert_eq!(
        edited, &expected_edit,
        "all unrelated authored fields must survive the bounded edit"
    );

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
    assert_eq!(
        f.ok(&["docs", "history", "show", "--id", &bundle])["status"],
        "FROZEN_SNAPSHOT"
    );
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
    // READY is a deterministic submission result. This test never dispatches an
    // author/reviewer or publishes the edit, so it does not assert reviewed UX-01
    // publication acceptance or bypass it using --unassessed.
}
