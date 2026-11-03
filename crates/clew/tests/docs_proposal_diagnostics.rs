//! Neutral public CLI regressions; no private inputs, model calls or publication.
#![cfg(unix)]
#[path = "support/documentation.rs"]
mod support;
use serde_json::{Value, json};
use std::{collections::BTreeSet, fs};
use support::{Fixture, commit};

fn receipt(name: &str, value: &Value) {
    if let Some(root) = std::env::var_os("CODECLEW_DIAGNOSTIC_ARTIFACT_DIR") {
        let root = std::path::PathBuf::from(root);
        fs::create_dir_all(&root).unwrap();
        fs::write(
            root.join(format!("{name}.json")),
            serde_json::to_vec_pretty(value).unwrap(),
        )
        .unwrap();
    }
}

fn read_page(f: &Fixture, work: &str, value: Value) -> Value {
    let input = f.input("selection.json", &value);
    f.ok(&[
        "docs",
        "work",
        "read",
        "--work",
        work,
        "--input",
        input.to_str().unwrap(),
    ])
}

fn initial(f: &Fixture, work: &str) -> Vec<Value> {
    let mut cursor = None;
    let mut rows = Vec::new();
    loop {
        let request = cursor.as_ref().map_or(json!({}), |c| json!({"cursor":c}));
        let page = read_page(f, work, request);
        assert!(page["omitted"].as_array().unwrap().is_empty());
        rows.extend(page["items"].as_array().unwrap().iter().cloned());
        cursor = page["nextCursor"].as_str().map(str::to_owned);
        if cursor.is_none() {
            break;
        }
    }
    rows
}

fn proposal_pages(f: &Fixture, mut page: Value) -> Vec<Value> {
    let proposal = page["proposal"].as_str().unwrap().to_owned();
    let mut rows = Vec::new();
    loop {
        assert!(page["omitted"].as_array().unwrap().is_empty());
        rows.extend(page["items"].as_array().unwrap().iter().cloned());
        let Some(cursor) = page["nextCursor"].as_str() else {
            break;
        };
        page = f.ok(&[
            "docs",
            "proposal",
            "show",
            "--proposal",
            &proposal,
            "--cursor",
            cursor,
        ]);
    }
    rows
}

fn submit(f: &Fixture, work: &str, input: &Value) -> Value {
    let path = f.input("proposal.json", input);
    f.ok(&[
        "docs",
        "proposal",
        "submit",
        "--work",
        work,
        "--input",
        path.to_str().unwrap(),
    ])
}

#[test]
fn proposal_missing_flows_are_complete_paginated_and_exact_lookup_is_recorded_offline() {
    let f = Fixture::new();
    let source = f.service("orders");
    let branches = (0..70)
        .map(|n| format!("if (value == {n}) value++; "))
        .collect::<String>();
    fs::write(source.join("Orders.java"),format!("public class Orders {{ public int first(int value) {{ {branches} return value; }} public int second(int value) {{ {branches} return value; }} }}")).unwrap();
    commit(&source);
    let checked = f.checked();
    let flow_ids: BTreeSet<_> = checked
        .dependencies
        .values()
        .filter(|row| {
            row.kind == "FLOW" && matches!(row.normalized["kind"].as_str(), Some("IF" | "RETURN"))
        })
        .map(|row| row.id.clone())
        .collect();
    assert!(flow_ids.len() > 128);
    let request=f.input("work.json",&json!({"schema":"codeclew-documentation-work-request/1.0","audience":"Owned diagnostics","maxItems":2,"maxBytes":49152}));
    let prepared = f.ok(&[
        "docs",
        "work",
        "prepare",
        "--subject",
        "service:orders",
        "--input",
        request.to_str().unwrap(),
    ]);
    let work = prepared["work"].as_str().unwrap();
    let initial = initial(&f, work);
    let entries: Vec<_> = initial
        .iter()
        .filter(|row| row["kind"] == "ENTRYPOINT")
        .collect();
    assert_eq!(entries.len(), 2);
    fs::rename(&source, source.with_extension("offline")).unwrap();
    let operations:Vec<_>=entries.iter().map(|row|json!({"entrypoint":row["reference"],"title":"Owned method","summary":{"text":"The method returns its value.","evidence":[row["reference"]]},"steps":[]})).collect();
    let input = json!({"schema":"codeclew-documentation-proposal/1.0","operations":operations});
    let result = submit(&f, work, &input);
    assert_eq!(result["status"], "NEEDS_REPAIR");
    let proposal = result["proposal"].as_str().unwrap().to_owned();
    let rows = proposal_pages(&f, result);
    let stored: Value = serde_json::from_slice(
        &fs::read(f.docs.join(format!(".codeclew/proposals/{proposal}.json"))).unwrap(),
    )
    .unwrap();
    assert!(stored["diagnosticsRef"].is_object());
    assert_eq!(stored["artifact"]["diagnostics"], json!([]));
    let plan = f.ok(&["docs", "storage", "plan"]);
    assert!(
        plan["plan"]["retained"]
            .get(stored["diagnosticsRef"]["digest"].as_str().unwrap())
            .is_some()
    );
    let missing: Vec<_> = rows
        .iter()
        .filter(|row| row["record"]["diagnosticCode"] == "MANDATORY_FLOW_UNCOVERED")
        .collect();
    let returned: BTreeSet<_> = missing
        .iter()
        .map(|row| row["record"]["flowId"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(returned, flow_ids);
    assert_eq!(missing.len(), flow_ids.len());
    let summary = rows
        .iter()
        .find(|row| row["record"]["code"] == "MANDATORY_FLOW_COVERAGE_INCOMPLETE")
        .unwrap();
    assert_eq!(summary["record"]["missingFlowCount"], flow_ids.len());
    assert!(
        missing
            .iter()
            .all(|row| row["record"]["referenceStatus"] == "SAME_WORK_HANDLE"
                && row["record"]["flowReferenceDelivered"] == false)
    );
    let ids: Vec<_> = flow_ids.iter().take(3).cloned().collect();
    receipt(
        "complete-missing-flows",
        &json!({"expectedCount":flow_ids.len(),"proposal":proposal,"diagnostics":missing.iter().map(|row|&row["record"]).collect::<Vec<_>>(),"summary":summary["record"],"cacheReference":stored["diagnosticsRef"]}),
    );
    let selection = json!({"query":{"kind":"FLOW","ids":ids,"expectedSnapshot":prepared["snapshot"],"expectedContextDigest":checked.context_digest}});
    let first = read_page(&f, work, selection.clone());
    assert_eq!(first["items"].as_array().unwrap().len(), 2);
    assert!(first["nextCursor"].is_string());
    let third = missing
        .iter()
        .find(|row| row["record"]["flowId"] == ids[2])
        .unwrap();
    let operation_index = entries
        .iter()
        .position(|row| row["record"]["id"] == third["record"]["operationId"])
        .unwrap();
    let mut unreceived = input.clone();
    unreceived["operations"][operation_index]["steps"] = json!([{"kind":"note","meaning":{"text":"Owned step.","evidence":[third["record"]["flowReference"]]}}]);
    let refused = proposal_pages(&f, submit(&f, work, &unreceived));
    assert!(refused.iter().any(|row| {
        row["record"]["code"] == "INVALID_PROPOSAL"
            && row["record"]["nextAction"]
                .as_str()
                .is_some_and(|s| s.contains("was not supplied"))
    }));
    let mut continued = selection.clone();
    continued["cursor"] = first["nextCursor"].clone();
    let next = read_page(&f, work, continued);
    assert_eq!(next["items"].as_array().unwrap().len(), 1);
    assert!(next["nextCursor"].is_null());
    assert_eq!(read_page(&f, work, selection), first);
    receipt("exact-identity-first-page", &first);
    let source_reference = first["items"][0]["sourceReferences"][0].as_str().unwrap();
    let request = json!({"references":[source_reference]});
    let original = read_page(&f, work, request.clone());
    assert!(
        original["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|row| row["kind"] == "SOURCE")
    );
    assert_eq!(read_page(&f, work, request), original);
    let again = f.ok(&["docs", "proposal", "show", "--proposal", &proposal]);
    assert_eq!(proposal_pages(&f, again), rows); // later reads do not rewrite saved diagnostics
    let bad = f.input(
        "stale-query.json",
        &json!({"query":{"kind":"FLOW","ids":[ids[0]],"expectedSnapshot":"foreign"}}),
    );
    let (code, error) = f.run(&[
        "docs",
        "work",
        "read",
        "--work",
        work,
        "--input",
        bad.to_str().unwrap(),
    ]);
    assert_eq!(code, 2);
    assert!(
        error["error"]["message"]
            .as_str()
            .unwrap()
            .contains("WORK_IDENTITY_SCOPE_MISMATCH")
    );
}

#[test]
fn proposal_schema_diagnostics_reuse_safe_paths_and_add_shapes_without_values() {
    let f = Fixture::new();
    let claim = json!({"text":"Owned meaning","evidence":["d0"]});
    let base = json!({"schema":"codeclew-documentation-proposal/1.0","operations":[{"entrypoint":"e0","title":"Owned","summary":claim,"steps":[{"kind":"note","meaning":claim}]}]});
    for (case, path, category, keyword, actual) in [
        (
            "extra",
            "$.operations[0].steps[0].evidence",
            "UNEXPECTED_FIELD",
            "additionalProperties",
            "present",
        ),
        (
            "missing",
            "$.operations[0].steps[0].meaning.evidence",
            "REQUIRED_FIELD_MISSING",
            "required",
            "absent",
        ),
        (
            "type",
            "$.operations[0].steps",
            "TYPE_OR_SHAPE_MISMATCH",
            "type",
            "integer",
        ),
        (
            "nested",
            "$.operations[0].contracts[0].rows[0].description.evidence",
            "TYPE_OR_SHAPE_MISMATCH",
            "type",
            "string",
        ),
    ] {
        let mut input = base.clone();
        match case {
            "extra" => {
                input["operations"][0]["steps"][0]["evidence"] = json!(["NEVER_PRINT_SECRET"])
            }
            "missing" => {
                input["operations"][0]["steps"][0]["meaning"]
                    .as_object_mut()
                    .unwrap()
                    .remove("evidence");
            }
            "type" => input["operations"][0]["steps"] = json!(7),
            _ => {
                input["operations"][0]["contracts"] = json!([{"title":"Owned","kind":"payload","rows":[{"label":"field","description":{"text":"Owned meaning","evidence":"NEVER_PRINT_SECRET"}}]}])
            }
        }
        let file = f.input("schema.json", &input);
        let (code, result) = f.run(&[
            "docs",
            "proposal",
            "submit",
            "--work",
            &"a".repeat(64),
            "--input",
            file.to_str().unwrap(),
        ]);
        assert_eq!(code, 2);
        assert_eq!(result["error"]["code"], "INVALID_INPUT");
        let detail = &result["error"]["details"];
        assert_eq!(detail["path"], path, "{result}");
        assert_eq!(detail["category"], category);
        assert_eq!(detail["keyword"], keyword);
        assert_eq!(detail["actualShape"], actual);
        assert!(detail["expectedShape"].is_string());
        assert!(detail["repair"].is_string());
        assert!(!result.to_string().contains("NEVER_PRINT_SECRET"));
        receipt(&format!("schema-{case}"), &result);
    }
    let mut input = base;
    input["operations"][0]["steps"][0]["PRIVATE_KEY_DO_NOT_PRINT"] = json!("NEVER_PRINT_SECRET");
    let file = f.input("unsafe-property.json", &input);
    let (_, result) = f.run(&[
        "docs",
        "proposal",
        "submit",
        "--work",
        &"a".repeat(64),
        "--input",
        file.to_str().unwrap(),
    ]);
    assert!(!result.to_string().contains("PRIVATE_KEY_DO_NOT_PRINT"));
    assert!(!result.to_string().contains("NEVER_PRINT_SECRET"));
    let malformed = f.temp.path().join("malformed.json");
    fs::write(&malformed, b"{\"operations\": [\"NEVER_PRINT_SECRET").unwrap();
    let (code, result) = f.run(&[
        "docs",
        "proposal",
        "submit",
        "--work",
        &"a".repeat(64),
        "--input",
        malformed.to_str().unwrap(),
    ]);
    assert_eq!(code, 2);
    assert_eq!(result["error"]["details"]["category"], "SYNTAX");
    assert_eq!(result["error"]["details"]["path"], "$");
    assert!(!result.to_string().contains("NEVER_PRINT_SECRET"));
    receipt("schema-malformed", &result);
}
