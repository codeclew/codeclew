//! Public source-profile capture and offline Work delivery, without a TypeScript compiler.
#![cfg(unix)]
#[path = "support/documentation.rs"]
mod support;
use serde_json::{Value, json};
use std::fs;
use support::{Fixture, commit, git};

#[test]
fn typescript_source_profile_captures_current_scope_and_reads_work_offline() {
    let fixture = Fixture::new();
    let checkout = fixture.service("dashboard");
    let text = "const TOPIC = 'weather.forecast-updated';\ntype Payload = { temperature: number };\nexport async function consume(value: Payload) { if (value.temperature === 0) return value; return await persist(TOPIC, value); }\n";
    fs::write(checkout.join("weather.ts"), text).unwrap();
    git(&checkout, &["add", "weather.ts"]);
    commit(&checkout);
    let record = fixture.input(
        "typescript-service.json",
        &json!({
            "schema":"codeclew-documentation-service/1.0","id":"dashboard","title":"Dashboard",
            "repositoryId":"dashboard","repository":"https://example.invalid/dashboard",
            "language":"typescript","profile":"source-syntax","targetRef":"HEAD",
            "source":{"roots":["weather.ts"],"dialect":"7.0.2"}
        }),
    );
    let digest = fixture.ok(&["docs", "service", "list"])["inputDigest"]
        .as_str()
        .unwrap()
        .to_owned();
    fixture.ok(&[
        "docs",
        "service",
        "add",
        "--input",
        record.to_str().unwrap(),
        "--expected-input-digest",
        &digest,
    ]);
    let (code, checked) = fixture.run(&["docs", "check"]);
    assert!(code == 0 || code == 3, "{checked}");
    let snapshot = checked["snapshot"].as_str().unwrap();
    let request = fixture.input("work.json", &json!({"schema":"codeclew-documentation-work-request/1.0","audience":"Maintainer","maxItems":100,"maxBytes":49152}));
    let prepared = fixture.ok(&[
        "docs",
        "work",
        "prepare",
        "--subject",
        "service:dashboard",
        "--snapshot",
        snapshot,
        "--input",
        request.to_str().unwrap(),
    ]);
    let work = prepared["work"].as_str().unwrap();
    fs::rename(&checkout, checkout.with_extension("offline")).unwrap();
    let selection = fixture.input("sources.json", &json!({"query":{"kind":"SOURCE"}}));
    let page = fixture.ok(&[
        "docs",
        "work",
        "read",
        "--work",
        work,
        "--input",
        selection.to_str().unwrap(),
    ]);
    assert_eq!(page["snapshot"], snapshot);
    let sources: Vec<&Value> = page["items"].as_array().unwrap().iter().collect();
    assert!(
        sources
            .iter()
            .any(|source| source["record"]["text"] == text),
        "{page}"
    );
}
