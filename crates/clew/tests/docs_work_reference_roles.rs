//! Work pages describe the proposal capabilities of each delivered handle.
#![cfg(unix)]

#[path = "support/documentation.rs"]
mod support;

use serde_json::{Value, json};
use std::fs;
use support::{Fixture, commit, read};

fn hydrated_work(f: &Fixture, id: &str) -> Value {
    let repo = clew::documentation::store::Repository::open(&f.docs).unwrap();
    serde_json::to_value(clew::documentation::work::load(&repo, id).unwrap()).unwrap()
}

fn request(
    f: &Fixture,
    name: &str,
    entrypoint: Option<&str>,
    max_bytes: usize,
) -> std::path::PathBuf {
    f.input(
        name,
        &json!({
            "schema":"codeclew-documentation-work-request/1.0",
            "audience":"Service maintainers",
            "entrypoint":entrypoint,
            "maxItems":100,
            "maxBytes":max_bytes
        }),
    )
}

fn read_all(f: &Fixture, mut page: Value, work: &str) -> Vec<Value> {
    let mut items = Vec::new();
    loop {
        items.extend(page["items"].as_array().unwrap().iter().cloned());
        let Some(cursor) = page["nextCursor"].as_str() else {
            break;
        };
        let input = f.input("roles-selection.json", &json!({"cursor":cursor}));
        page = f.ok(&[
            "docs",
            "work",
            "read",
            "--work",
            work,
            "--input",
            input.to_str().unwrap(),
        ]);
    }
    items
}

#[test]
fn delivered_items_advertise_only_eligible_reference_roles() {
    let f = Fixture::new();
    f.service("orders");
    let checked = f.checked();
    let entrypoint = checked.services["orders"].entrypoints[0].id.clone();

    let request_path = request(&f, "roles-request.json", None, 49_152);
    let page = f.ok(&[
        "docs",
        "work",
        "prepare",
        "--subject",
        "service:orders",
        "--input",
        request_path.to_str().unwrap(),
    ]);
    let work = page["work"].as_str().unwrap().to_owned();
    let items = read_all(&f, page, &work);

    assert!(items.iter().all(|item| item["referenceRoles"].is_array()));
    assert!(items.iter().any(|item| {
        item["kind"] == "SECTION" && item["referenceRoles"] == json!(["operation", "gap"])
    }));
    assert!(items.iter().any(|item| {
        item["kind"] == "COVERAGE"
            && item["reference"].is_null()
            && item["referenceRoles"] == json!([])
    }));
    let frozen = hydrated_work(&f, &work);
    let source_reference = frozen["handles"]
        .as_object()
        .unwrap()
        .iter()
        .find(|(_, handle)| handle["kind"] == "SOURCE")
        .map(|(reference, _)| reference.clone())
        .unwrap();
    let source_input = f.input(
        "source-selection.json",
        &json!({"references":[source_reference]}),
    );
    let source_page = f.ok(&[
        "docs",
        "work",
        "read",
        "--work",
        &work,
        "--input",
        source_input.to_str().unwrap(),
    ]);
    let source_item = source_page["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["kind"] == "SOURCE")
        .unwrap();
    assert_eq!(source_item["referenceRoles"], json!(["evidence"]));

    let requested_path = request(
        &f,
        "requested-roles-request.json",
        Some(&entrypoint),
        49_152,
    );
    let requested_page = f.ok(&[
        "docs",
        "work",
        "prepare",
        "--subject",
        "service:orders",
        "--input",
        requested_path.to_str().unwrap(),
    ]);
    let requested_work = requested_page["work"].as_str().unwrap().to_owned();
    let requested_items = read_all(&f, requested_page, &requested_work);
    assert!(requested_items.iter().any(|item| {
        item["kind"] == "SECTION"
            && !item["referenceRoles"]
                .as_array()
                .unwrap()
                .iter()
                .any(|role| role == "operation")
    }));

    // Keep the assertion grounded in the persisted read contract: role
    // metadata is page data, while only supplied handles enter the receipt.
    let ledger: Value = serde_json::from_slice(
        &fs::read(f.docs.join(format!(".codeclew/work/{work}/reads.json"))).unwrap(),
    )
    .unwrap();
    assert!(
        ledger["receipts"]
            .as_object()
            .unwrap()
            .values()
            .all(|receipt| {
                receipt["supplied"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .all(|reference| reference.as_str().is_some_and(|r| !r.is_empty()))
            })
    );
}

#[test]
fn omitted_role_rows_are_not_recorded_as_supplied() {
    let f = Fixture::new();
    let source = f.service("orders");
    fs::write(
        source.join("Orders.java"),
        format!(
            "public class Orders {{ public int reserve(int quantity) {{\n{}return quantity; }} }}\n",
            "// large source line\n".repeat(1800)
        ),
    )
    .unwrap();
    commit(&source);
    // Authoring consumes saved evidence; it never captures this changed source.
    f.checked();
    let request_path = request(&f, "omitted-roles-request.json", None, 2_048);
    let page = f.ok(&[
        "docs",
        "work",
        "prepare",
        "--subject",
        "service:orders",
        "--input",
        request_path.to_str().unwrap(),
    ]);
    let work = page["work"].as_str().unwrap().to_owned();
    let frozen = hydrated_work(&f, &work);
    let source_reference = frozen["handles"]
        .as_object()
        .unwrap()
        .iter()
        .find(|(_, handle)| handle["kind"] == "SOURCE")
        .map(|(reference, _)| reference.clone())
        .unwrap();
    let selection = f.input(
        "omitted-source-selection.json",
        &json!({"references":[source_reference]}),
    );
    let mut source_page = f.ok(&[
        "docs",
        "work",
        "read",
        "--work",
        &work,
        "--input",
        selection.to_str().unwrap(),
    ]);
    let omitted = loop {
        if let Some(row) = source_page["omitted"]
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["reference"].is_string())
        {
            break row.clone();
        }
        let Some(cursor) = source_page["nextCursor"].as_str().map(str::to_owned) else {
            panic!("large handled source was never omitted");
        };
        let input = f.input(
            "omitted-source-cursor.json",
            &json!({"references":[source_reference], "cursor":cursor}),
        );
        source_page = f.ok(&[
            "docs",
            "work",
            "read",
            "--work",
            &work,
            "--input",
            input.to_str().unwrap(),
        ]);
    };
    assert!(omitted["referenceRoles"].is_null());
    let ledger = read(f.docs.join(format!(".codeclew/work/{work}/reads.json")));
    assert!(
        ledger["receipts"]
            .as_object()
            .unwrap()
            .values()
            .all(|receipt| {
                !receipt["supplied"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|reference| reference == &omitted["reference"])
            })
    );
}

#[test]
fn scenario_subject_page_exposes_operation_and_gap_target() {
    let f = Fixture::new();
    f.service("orders");
    fs::write(
        f.docs.join("scenarios/reserve.yaml"),
        serde_json::to_vec(&json!({
            "schema":"codeclew-documentation-process/1.0",
            "interactions": [], "maxDepth":4, "maxNodes":64,
            "process":{"scope":"Declared quantity handling.", "participants":["orders"], "objects":[], "trigger":"A request arrives.", "outcomes":["A quantity is returned."], "linkedSubviews":[]},
            "id":"reserve",
            "title":"Reserve quantity",
            "summary":"Explicit saved selection",
            "root":{
                "service":"orders",
                "selector":{
                    "language":"java",
                    "owner":"Orders",
                    "name":"reserve",
                    "parameterTypes":["int"]
                }
            }
        }))
        .unwrap(),
    )
    .unwrap();
    f.checked();
    let request_path = request(
        &f,
        "scenario-roles-request.json",
        Some("process-overview"),
        49_152,
    );
    let mut page = f.ok(&[
        "docs",
        "work",
        "prepare",
        "--subject",
        "scenario:reserve",
        "--input",
        request_path.to_str().unwrap(),
    ]);
    let work = page["work"].as_str().unwrap().to_owned();
    assert_eq!(
        page["subjectReference"]["referenceRoles"],
        json!(["operation", "gap"])
    );
    let initial_roots = page["items"].as_array().unwrap();
    assert!(initial_roots.iter().any(|item| {
        item["kind"] == "PROCESS_ROOT"
            && item["id"] == "process-overview"
            && item["referenceRoles"] == json!(["operation", "gap"])
    }));
    assert!(initial_roots.iter().any(|item| {
        item["kind"] == "PROCESS_ROOT"
            && item["id"] == "reserve"
            && item["referenceRoles"] == json!(["gap"])
    }));
    while let Some(cursor) = page["nextCursor"].as_str().map(str::to_owned) {
        let input = f.input("scenario-roles-selection.json", &json!({"cursor":cursor}));
        page = f.ok(&[
            "docs",
            "work",
            "read",
            "--work",
            &work,
            "--input",
            input.to_str().unwrap(),
        ]);
    }
}
