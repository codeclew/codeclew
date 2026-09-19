//! Work pages describe the proposal capabilities of each delivered handle.
#![cfg(unix)]

#[path = "support/documentation.rs"]
mod support;

use serde_json::{Value, json};
use std::fs;
use support::{Fixture, commit, read};

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
    let frozen = read(f.docs.join(format!(".codeclew/work/{work}/work.json")));
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

fn submit(f: &Fixture, work: &str, input: &Value, name: &str) -> (i32, Value) {
    let path = f.input(name, input);
    f.run(&[
        "docs",
        "proposal",
        "submit",
        "--work",
        work,
        "--input",
        path.to_str().unwrap(),
    ])
}

fn submit_ready(f: &Fixture, work: &str, input: &Value, name: &str) {
    let (code, result) = submit(f, work, input, name);
    assert_eq!(code, 0, "{result}");
    assert!(
        matches!(
            result["status"].as_str(),
            Some("READY_FOR_REVIEW" | "READY_WITH_LIMITATIONS")
        ),
        "{result}"
    );
}

fn submit_invalid(f: &Fixture, work: &str, input: &Value, name: &str, message: &str) {
    let (_, result) = submit(f, work, input, name);
    assert_eq!(result["status"], "NEEDS_REPAIR", "{result}");
    assert!(
        result["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|diagnostic| {
                diagnostic["kind"] == "DIAGNOSTIC"
                    && diagnostic["record"]["code"] == "INVALID_PROPOSAL"
                    && diagnostic["record"]["nextAction"]
                        .as_str()
                        .is_some_and(|actual| actual.contains(message))
            }),
        "{result}"
    );
}

fn proposal(entrypoint: &str, evidence: &str) -> Value {
    json!({
        "schema":"codeclew-documentation-proposal/1.0",
        "operations":[{
            "entrypoint":entrypoint,
            "title":"Reserve quantity",
            "summary":{
                "text":"The selected operation processes a requested quantity.",
                "evidence":[evidence]
            },
            "steps":[]
        }]
    })
}

#[test]
fn proposal_submission_uses_handles_and_role_predicates() {
    let f = Fixture::new();
    f.service("orders");
    let checked = f.checked();
    let entrypoint = "section-entities".to_owned();
    let request_path = request(&f, "proposal-roles-request.json", Some(&entrypoint), 49_152);
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
    let initial_items = read_all(&f, page, &work);
    let frozen = read(f.docs.join(format!(".codeclew/work/{work}/work.json")));
    let handle = |kind: &str, id: Option<&str>| {
        frozen["handles"]
            .as_object()
            .unwrap()
            .iter()
            .find(|(_, value)| {
                value["kind"] == kind && id.is_none_or(|wanted| value["id"] == wanted)
            })
            .map(|(reference, _)| reference.clone())
            .unwrap()
    };
    let entry_ref = handle("SECTION", Some(&entrypoint));
    let evidence_ref = handle("ENTRYPOINT", None);
    let section_ref = handle("SECTION", None);
    let source_ref = handle("SOURCE", None);
    let scope_ref = frozen["handles"]
        .as_object()
        .unwrap()
        .iter()
        .find(|(_, value)| {
            value["kind"] == "DEPENDENCY"
                && checked.dependencies[value["id"].as_str().unwrap()].kind == "SOURCE_SCOPE"
        })
        .map(|(reference, _)| reference.clone())
        .unwrap();

    let evidence_selection = f.input(
        "positive-evidence.json",
        &json!({"references":[evidence_ref]}),
    );
    f.ok(&[
        "docs",
        "work",
        "read",
        "--work",
        &work,
        "--input",
        evidence_selection.to_str().unwrap(),
    ]);
    let valid = proposal(&entry_ref, &evidence_ref);
    submit_ready(&f, &work, &valid, "proposal-valid.json");

    for (name, evidence) in [
        ("proposal-raw-obligation.json", "obligation-1"),
        ("proposal-raw-section.json", "section-overview"),
    ] {
        let mut invalid = valid.clone();
        invalid["operations"][0]["summary"]["evidence"] = json!([evidence]);
        submit_invalid(&f, &work, &invalid, name, "unknown work reference");
    }

    let mut section_evidence = valid.clone();
    section_evidence["operations"][0]["summary"]["evidence"] = json!([section_ref]);
    submit_invalid(
        &f,
        &work,
        &section_evidence,
        "proposal-section-evidence.json",
        "unsupported evidence reference",
    );

    // Merely listing a sourceReferences expansion link must not grant a read.
    let ledger = read(f.docs.join(format!(".codeclew/work/{work}/reads.json")));
    let received: std::collections::BTreeSet<_> = ledger["receipts"]
        .as_object()
        .unwrap()
        .values()
        .flat_map(|receipt| receipt["supplied"].as_array().unwrap())
        .filter_map(Value::as_str)
        .collect();
    let unread_source = initial_items
        .iter()
        .filter_map(|item| item["sourceReferences"].as_array())
        .flatten()
        .filter_map(Value::as_str)
        .find(|reference| !received.contains(reference))
        .expect("fixture must expose an expansion link to an unread source");
    let mut unread = valid.clone();
    unread["operations"][0]["summary"]["evidence"] = json!([unread_source]);
    submit_invalid(
        &f,
        &work,
        &unread,
        "proposal-unread-source.json",
        "was not supplied by a recorded work read",
    );

    let source_input = f.input(
        "proposal-source-selection.json",
        &json!({"references":[source_ref]}),
    );
    f.ok(&[
        "docs",
        "work",
        "read",
        "--work",
        &work,
        "--input",
        source_input.to_str().unwrap(),
    ]);
    let mut source_operation = valid.clone();
    source_operation["operations"][0]["entrypoint"] = json!(source_ref);
    submit_invalid(
        &f,
        &work,
        &source_operation,
        "proposal-source-operation.json",
        "operation requires an entrypoint work reference",
    );
    let mut source_gap = valid.clone();
    source_gap["gaps"] = json!({source_ref:"Source is not an operation or gap target."});
    submit_invalid(
        &f,
        &work,
        &source_gap,
        "proposal-source-gap.json",
        "gap requires an entrypoint reference",
    );

    let mut other_section = valid.clone();
    other_section["operations"][0]["entrypoint"] = json!(section_ref);
    submit_invalid(
        &f,
        &work,
        &other_section,
        "proposal-other-section.json",
        "operation is outside the requested entrypoint",
    );

    let mut scope_only = valid;
    scope_only["operations"][0]["summary"]["evidence"] = json!([scope_ref]);
    submit_invalid(
        &f,
        &work,
        &scope_only,
        "proposal-scope-only.json",
        "claim evidence exceeds canonical bounds",
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
    let frozen = read(f.docs.join(format!(".codeclew/work/{work}/work.json")));
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
    let checked = f.checked();
    let dependency = checked
        .dependencies
        .values()
        .find(|observation| observation.kind == "FLOW" && !observation.source_ids.is_empty())
        .unwrap();
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
    let frozen = read(f.docs.join(format!(".codeclew/work/{work}/work.json")));
    let evidence = frozen["handles"]
        .as_object()
        .unwrap()
        .iter()
        .find(|(_, handle)| handle["id"] == dependency.id)
        .map(|(reference, _)| reference.clone())
        .unwrap();
    let scenario_proposal = proposal("scenario:reserve", &evidence);
    submit_ready(
        &f,
        &work,
        &scenario_proposal,
        "scenario-roles-proposal.json",
    );
}
