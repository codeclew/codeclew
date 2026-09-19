//! A compact process view cannot authorize provider facts hidden by its projection.
#![cfg(unix)]
#[path = "support/documentation.rs"]
mod support;
use serde_json::json;
use support::Fixture;

#[test]
fn process_profile_defers_provider_authority_until_recorded_expansion() {
    let f = Fixture::new();
    let source = f.service("orders");
    let definition=f.input("process.json",&json!({"schema":"codeclew-documentation-process/1.0","id":"quantity","title":"Quantity flow","summary":"One explicitly selected method.","root":{"service":"orders","selector":{"language":"java","owner":"Orders","name":"reserve","parameterTypes":["int"]}},"interactions":[],"maxDepth":3,"maxNodes":32,"process":{"scope":"One method","participants":["orders"],"objects":[],"trigger":"An unspecified caller provides quantity.","outcomes":["Returns a quantity."],"linkedSubviews":[]}}));
    let before = f.ok(&["docs", "process", "list"]);
    f.ok(&[
        "docs",
        "process",
        "put",
        "--input",
        definition.to_str().unwrap(),
        "--expected-input-digest",
        before["inputDigest"].as_str().unwrap(),
    ]);
    f.checked();
    std::fs::rename(&source, source.with_extension("offline")).unwrap();
    let first = f.ok(&[
        "docs",
        "process",
        "prepare",
        "--id",
        "quantity",
        "--overview",
    ]);
    assert_eq!(first["contextProfile"], "process-v1");
    let work = first["work"].as_str().unwrap().to_owned();
    let mut page = first;
    let mut rows = Vec::new();
    loop {
        assert!(page["omitted"].as_array().unwrap().is_empty());
        rows.extend(page["items"].as_array().unwrap().iter().cloned());
        let Some(cursor) = page["nextCursor"].as_str() else {
            break;
        };
        let selection = f.input("next-page.json", &json!({"cursor":cursor}));
        page = f.ok(&[
            "docs",
            "work",
            "read",
            "--work",
            &work,
            "--input",
            selection.to_str().unwrap(),
        ]);
    }
    let deferred = rows
        .iter()
        .find(|row| row["kind"] == "CALLABLE_SUMMARY")
        .unwrap()["record"]["fullRecordReference"]
        .as_str()
        .unwrap();
    assert!(!rows.iter().any(|row| row["reference"] == deferred));
    let proposal=f.input("proposal.json",&json!({"schema":"codeclew-documentation-proposal/1.0","operations":[{"entrypoint":"scenario:quantity","title":"Quantity method","summary":{"text":"The selected method returns a quantity.","evidence":[deferred]},"steps":[]}],"gaps":{},"uncertainties":["Call targets and runtime activation are not established by syntax evidence."]}));
    let rejected = f.ok(&[
        "docs",
        "proposal",
        "submit",
        "--work",
        &work,
        "--input",
        proposal.to_str().unwrap(),
    ]);
    assert_eq!(rejected["status"], "NEEDS_REPAIR", "{rejected}");
    assert!(rejected.to_string().contains("not supplied"), "{rejected}");
    let selection = f.input("expand.json", &json!({"references":[deferred]}));
    let expanded = f.ok(&[
        "docs",
        "work",
        "expand",
        "--work",
        &work,
        "--input",
        selection.to_str().unwrap(),
    ]);
    let full = expanded["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["reference"] == deferred)
        .unwrap();
    assert_eq!(full["record"]["kind"], "SYMBOL");
    assert!(full["record"]["normalized"].get("documentation").is_some());
    let accepted = f.ok(&[
        "docs",
        "proposal",
        "submit",
        "--work",
        &work,
        "--input",
        proposal.to_str().unwrap(),
    ]);
    assert!(
        accepted["status"].as_str().unwrap().starts_with("READY_"),
        "{accepted}"
    );
    // The overview host contract accepts the supported summary above but not
    // sequence, contract, note-assessment or typed-view content in that slot.
    let good: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&proposal).unwrap()).unwrap();
    for (field, value, diagnostic) in [
        (
            "steps",
            json!([{"kind":"note","meaning":{"text":"A step","evidence":[deferred]}}]),
            "section proposals use a supported summary",
        ),
        (
            "contracts",
            json!([{"title":"Payload","kind":"payload","rows":[{"label":"Quantity","description":{"text":"A quantity","evidence":[deferred]}}]}]),
            "section proposals use a supported summary",
        ),
        (
            "assessment",
            json!({"outcome":"UNKNOWN","period":"Current"}),
            "assessments require an explicit note root",
        ),
        (
            "dataflow",
            json!({"nodes":[],"edges":[]}),
            "typed data-flow content requires a saved view root",
        ),
    ] {
        let mut invalid = good.clone();
        invalid["operations"][0][field] = value;
        let path = f.input(&format!("invalid-overview-{field}.json"), &invalid);
        let rejected = f.ok(&[
            "docs",
            "proposal",
            "submit",
            "--work",
            &work,
            "--input",
            path.to_str().unwrap(),
        ]);
        assert_eq!(rejected["status"], "NEEDS_REPAIR", "{rejected}");
        assert!(rejected.to_string().contains(diagnostic), "{rejected}");
    }
    for key in ["activation", "provider-behavior", "runtime"] {
        let mut invalid = good.clone();
        invalid["gaps"] = json!({(key):"This limitation is not an operation root."});
        let path = f.input(&format!("invalid-gap-{key}.json"), &invalid);
        let rejected = f.ok(&[
            "docs",
            "proposal",
            "submit",
            "--work",
            &work,
            "--input",
            path.to_str().unwrap(),
        ]);
        assert_eq!(rejected["status"], "NEEDS_REPAIR", "{rejected}");
        assert!(
            rejected
                .to_string()
                .contains("gap requires an entrypoint reference or its scenario subject"),
            "{rejected}"
        );
    }
    let mut duplicate = good.clone();
    duplicate["gaps"] = json!({"scenario:quantity":"Runtime activation is not established."});
    let path = f.input("duplicate-overview-gap.json", &duplicate);
    let rejected = f.ok(&[
        "docs",
        "proposal",
        "submit",
        "--work",
        &work,
        "--input",
        path.to_str().unwrap(),
    ]);
    assert_eq!(rejected["status"], "NEEDS_REPAIR", "{rejected}");
    assert!(
        rejected
            .to_string()
            .contains("gap is duplicate, out of scope, or lacks an actionable reason"),
        "{rejected}"
    );
    let mut supported = good;
    supported["operations"][0]["summary"]["uncertainty"] =
        json!("Runtime activation and provider behavior remain unverified.");
    let path = f.input("supported-overview-uncertainties.json", &supported);
    let limited = f.ok(&[
        "docs",
        "proposal",
        "submit",
        "--work",
        &work,
        "--input",
        path.to_str().unwrap(),
    ]);
    assert_eq!(limited["status"], "READY_WITH_LIMITATIONS", "{limited}");
    let path = f.input("unsupported-whole-overview.json", &json!({
        "schema":"codeclew-documentation-proposal/1.0", "operations":[],
        "gaps":{"scenario:quantity":"No supported overview is offered; further retained evidence is needed."},
    }));
    let unavailable = f.ok(&[
        "docs",
        "proposal",
        "submit",
        "--work",
        &work,
        "--input",
        path.to_str().unwrap(),
    ]);
    assert_eq!(
        unavailable["status"], "READY_WITH_LIMITATIONS",
        "{unavailable}"
    );
}

#[test]
fn process_profile_cannot_be_requested_for_service_sections() {
    let f = Fixture::new();
    f.service("orders");
    f.checked();
    let request=f.input("request.json",&json!({"schema":"codeclew-documentation-work-request/1.0","audience":"Maintainers","entrypoint":"section-overview","contextProfile":"process-v1"}));
    let (code, error) = f.run(&[
        "docs",
        "work",
        "prepare",
        "--subject",
        "service:orders",
        "--input",
        request.to_str().unwrap(),
    ]);
    assert_ne!(code, 0);
    assert!(
        error.to_string().contains("CONTEXT_PROFILE_INCOMPATIBLE"),
        "{error}"
    );
}
