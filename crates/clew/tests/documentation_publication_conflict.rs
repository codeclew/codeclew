#![cfg(unix)]
#[path = "support/documentation.rs"]
mod support;

use clew::documentation::{bindings, check::Check, store::Repository, work};
use serde_json::{Value, json};
use support::{Fixture, read};

fn prepared_proposal(f: &Fixture, checked: &Check, service: &str) -> (String, Value) {
    let entry = checked.services[service]
        .entrypoints
        .iter()
        .find(|entry| entry.symbol.contains("reserve"))
        .unwrap();
    let repo = Repository::open(&f.docs).unwrap();
    let snapshot = checked.save_snapshot(&repo).unwrap();
    let request = f.input(
        "request.json",
        &json!({
            "schema":"codeclew-documentation-work-request/1.0",
            "audience":"Service maintainers", "entrypoint":entry.id,
            "maxItems":100,"maxBytes":49152
        }),
    );
    let subject = format!("service:{service}");
    let mut page = f.ok(&[
        "docs",
        "work",
        "prepare",
        "--subject",
        &subject,
        "--snapshot",
        &snapshot,
        "--input",
        request.to_str().unwrap(),
    ]);
    let id = page["work"].as_str().unwrap().to_owned();
    while let Some(cursor) = page["nextCursor"].as_str() {
        let selection = f.input("selection.json", &json!({"cursor":cursor}));
        page = f.ok(&[
            "docs",
            "work",
            "expand",
            "--work",
            &id,
            "--input",
            selection.to_str().unwrap(),
        ]);
    }
    let frozen = work::load(&repo, &id).unwrap();
    assert!(frozen.retained.is_none());
    let reference = frozen
        .handles
        .iter()
        .find(|(_, h)| h.id == entry.id)
        .unwrap()
        .0;
    let returned = checked.services[service]
        .observations
        .values()
        .find(|d| d.kind == "FLOW" && d.symbol == entry.symbol && d.normalized["kind"] == "RETURN")
        .unwrap();
    let return_reference = frozen
        .handles
        .iter()
        .find(|(_, h)| h.id == returned.id)
        .unwrap()
        .0;
    let proposal = json!({
        "schema":"codeclew-documentation-proposal/1.0",
        "operations":[{"entrypoint":reference,"title":format!("Reserve for {service}"),
            "summary":{"text":"Processes the requested quantity.","evidence":[reference]},
            "steps":[{"kind":"note","meaning":{"text":"Returns the resulting quantity.","evidence":[return_reference]}}]}]
    });
    (id, proposal)
}

fn submit(f: &Fixture, work: &str, proposal: &Value) -> String {
    let input = f.input("proposal.json", proposal);
    let submitted = f.ok(&[
        "docs",
        "proposal",
        "submit",
        "--work",
        work,
        "--input",
        input.to_str().unwrap(),
    ]);
    assert!(
        submitted["status"].as_str().unwrap().starts_with("READY_"),
        "{submitted}"
    );
    submitted["proposal"].as_str().unwrap().to_owned()
}

#[test]
fn independently_prepared_services_publish_through_generated_placeholders_but_not_authored_changes()
{
    let f = Fixture::new();
    f.service("orders");
    f.service("shipping");
    let checked = f.checked();
    let (orders_work, orders_input) = prepared_proposal(&f, &checked, "orders");
    let (shipping_work, shipping_input) = prepared_proposal(&f, &checked, "shipping");
    let orders = submit(&f, &orders_work, &orders_input);
    let shipping = submit(&f, &shipping_work, &shipping_input);
    let mut competing_input = shipping_input.clone();
    competing_input["operations"][0]["summary"]["text"] =
        json!("An independently authored explanation.");
    let competing = submit(&f, &shipping_work, &competing_input);
    let first = f.ok(&[
        "docs",
        "proposal",
        "publish",
        "--proposal",
        &orders,
        "--unassessed",
    ]);
    let first_binding = read(f.bundle(first["bundle"].as_str().unwrap(), "bindings.json"));
    assert_eq!(
        first_binding["narratives"]["service:shipping"]["operations"],
        json!([])
    );
    // The same saved Work remains usable for submission as well as publication.
    assert_eq!(submit(&f, &shipping_work, &shipping_input), shipping);
    let second = f.ok(&[
        "docs",
        "proposal",
        "publish",
        "--proposal",
        &shipping,
        "--unassessed",
    ]);
    assert_eq!(second["updateFailures"], json!({}));
    let second_binding = read(f.bundle(second["bundle"].as_str().unwrap(), "bindings.json"));
    assert_eq!(
        second_binding["narratives"]["service:orders"],
        first_binding["narratives"]["service:orders"]
    );
    assert_eq!(
        second_binding["narratives"]["service:shipping"]["operations"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    let (code, rejected) = f.run(&[
        "docs",
        "proposal",
        "publish",
        "--proposal",
        &competing,
        "--unassessed",
    ]);
    assert_ne!(code, 0, "{rejected}");
    assert!(rejected.to_string().contains("WW_CONFLICT"), "{rejected}");
    let repo = Repository::open(&f.docs).unwrap();
    assert_eq!(
        bindings::baseline(&repo).unwrap().unwrap().0,
        second["bundle"].as_str().unwrap()
    );
}

#[test]
fn authored_gap_is_not_treated_as_a_generated_placeholder() {
    let f = Fixture::new();
    f.service("orders");
    let checked = f.checked();
    let (work, input) = prepared_proposal(&f, &checked, "orders");
    let proposal = submit(&f, &work, &input);
    let mut narrative = read(f.author("orders", &checked));
    let operation = narrative["operations"][0]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    narrative["operations"] = json!([]);
    narrative["gaps"][&operation] =
        json!("Maintainer deliberately deferred this operation pending contract review.");
    let path = f.input("gap.json", &narrative);
    let published = f.ok(&["docs", "render", "--input", path.to_str().unwrap()]);
    assert_eq!(published["updateFailures"], json!({}));
    let (code, rejected) = f.run(&[
        "docs",
        "proposal",
        "publish",
        "--proposal",
        &proposal,
        "--unassessed",
    ]);
    assert_ne!(code, 0, "{rejected}");
    assert!(rejected.to_string().contains("WW_CONFLICT"), "{rejected}");
}
