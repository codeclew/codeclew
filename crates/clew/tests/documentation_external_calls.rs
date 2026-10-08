#![cfg(unix)]
#[path = "support/documentation.rs"]
mod support;
use clew::documentation::{check::Check, store::Repository};
use serde_json::{Value, json};
use std::fs;
use support::{Fixture, commit, read};

#[test]
fn external_call_cli_recomposes_without_receiver_checkout_and_retains_details() {
    let f = Fixture::new();
    let caller = f.service("orders");
    fs::write(caller.join("Orders.java"), "public class Orders { public int reserve(int quantity) { return client.reserve(quantity); } }\n").unwrap();
    fs::write(caller.join("api.json"), serde_json::to_vec(&json!({
        "openapi":"3.1.2","info":{"title":"Synthetic declarations","version":"1"},
        "paths":{
            "/incoming-orders":{"post":{"operationId":"incoming","responses":{"200":{"description":"Incoming order accepted"}}}},
            "/external-reservations":{"post":{"operationId":"outgoing","requestBody":{"required":true,"content":{"application/json":{"schema":{"type":"object","properties":{"quantity":{"type":"integer"}},"required":["quantity"]}}}},"responses":{"201":{"description":"External reservation created","content":{"application/json":{"schema":{"type":"object","properties":{"reservationId":{"type":"string"}}}}}}}}}
        }
    })).unwrap()).unwrap();
    commit(&caller);
    let mut service = read(f.docs.join("catalog/services/orders.json"));
    service["contractFiles"] = json!(["api.json"]);
    let input = f.input("service-contracts.json", &service);
    let digest = f.ok(&["docs", "service", "list"])["inputDigest"]
        .as_str()
        .unwrap()
        .to_owned();
    f.ok(&[
        "docs",
        "service",
        "add",
        "--input",
        input.to_str().unwrap(),
        "--expected-input-digest",
        &digest,
    ]);
    let captured = f.checked();
    assert_eq!(captured.services.len(), 1);
    let repo = Repository::open(&f.docs).unwrap();
    let original = captured.save_snapshot(&repo).unwrap();
    fs::remove_dir_all(&caller).unwrap();
    let outgoing = captured
        .dependencies
        .values()
        .find(|o| {
            o.kind == "CONTRACT_OPERATION" && o.normalized["path"] == "/external-reservations"
        })
        .unwrap();
    let incoming = captured
        .dependencies
        .values()
        .find(|o| o.kind == "CONTRACT_OPERATION" && o.normalized["path"] == "/incoming-orders")
        .unwrap();
    let declaration = captured
        .dependencies
        .values()
        .find(|o| o.kind == "SYMBOL" && o.normalized["name"] == "reserve")
        .unwrap();
    let call = captured
        .dependencies
        .values()
        .find(|o| {
            o.kind == "FLOW" && o.symbol == declaration.symbol && o.normalized["kind"] == "CALL"
        })
        .unwrap();
    let mut interaction = json!({
        "schema":"codeclew-documentation-interaction/1.0","id":"reserve-external","title":"Reserve at the external inventory service",
        "external":true,"from":{"service":"orders","selector":{"language":"java","owner":declaration.normalized["ownerIdentity"],"name":"reserve"},"callSite":{"observation":call.id}},
        "to":{"service":"external-inventory"},"transport":{"kind":"http","method":"POST","path":"/external-reservations"},
        "declaration":{"origin":"human","rationale":"Synthetic operator-provided client binding; execution is not verified."},
        "addresses":[{"url":"https://inventory.example.invalid/v1","environment":"staging","source":"Synthetic deployment inventory"}],
        "applicability":{"environments":["staging"]},"contractReference":outgoing.id,
    });
    let put = |value: &Value| {
        let input = f.input("interaction.json", value);
        let digest = f.ok(&["docs", "interaction", "list"])["inputDigest"]
            .as_str()
            .unwrap()
            .to_owned();
        f.ok(&[
            "docs",
            "interaction",
            "put",
            "--input",
            input.to_str().unwrap(),
            "--expected-input-digest",
            &digest,
        ])
    };
    put(&interaction);
    let mut without_environment = interaction.clone();
    without_environment["addresses"][0]
        .as_object_mut()
        .unwrap()
        .remove("environment");
    put(&without_environment);
    put(&interaction);
    assert_eq!(repo.services().unwrap().len(), 1);
    assert!(
        !f.docs
            .join("catalog/services/external-inventory.json")
            .exists()
    );
    let (code, composed) = f.run(&["docs", "recompose", "--snapshot", &original]);
    assert!(matches!(code, 0 | 3), "{composed}");
    let snapshot = composed["snapshot"].as_str().unwrap();
    let candidate_input = f.input("candidate-interaction.json", &interaction);
    let candidate = f.ok(&[
        "docs",
        "interaction",
        "candidates",
        "--input",
        candidate_input.to_str().unwrap(),
        "--snapshot",
        snapshot,
    ]);
    assert_eq!(candidate["result"]["callSite"]["status"], "RESOLVED");
    assert_eq!(candidate["result"]["to"]["status"], "INCOMPLETE");
    let checked = Check::load_snapshot(&repo, snapshot).unwrap();
    assert_eq!(checked.services.len(), 1);
    assert!(checked.unresolved.is_empty());
    assert_eq!(
        checked.interactions["reserve-external"].from.status,
        "SOURCE_MATCH"
    );
    assert_eq!(
        checked.interactions["reserve-external"].call_site.status,
        "RESOLVED"
    );
    assert_eq!(
        checked.interactions["reserve-external"].to.status,
        "INCOMPLETE"
    );
    let mut wrong_site: clew::documentation::model::Interaction =
        serde_json::from_value(interaction.clone()).unwrap();
    wrong_site.from.call_site.as_mut().unwrap().observation = Some(incoming.id.clone());
    assert_eq!(
        clew::documentation::check::check_interaction(&wrong_site, &checked.services)
            .unwrap()
            .call_site
            .status,
        "MISSING"
    );
    let path = f.author("orders", &checked);
    let mut narrative = read(&path);
    narrative["operations"][0]["title"] = json!("Synthetic external inventory call");
    narrative["operations"][0]["participants"]
        .as_array_mut()
        .unwrap()
        .push(json!({"id":"external-client","label":"External inventory client","service":null}));
    let event = narrative["operations"][0]["events"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|e| {
            e["dependencyIds"]
                .as_array()
                .unwrap()
                .iter()
                .any(|id| id == &call.id)
        })
        .unwrap();
    event["kind"] = json!("message");
    event["from"] = json!("service");
    event["to"] = json!("external-client");
    event["text"] = json!("Request an external inventory reservation.");
    let event_id = event["id"].as_str().unwrap().to_owned();
    let operation_id = narrative["operations"][0]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    fs::write(&path, serde_json::to_vec(&narrative).unwrap()).unwrap();
    let rendered = f.ok(&[
        "docs",
        "render",
        "--snapshot",
        snapshot,
        "--input",
        path.to_str().unwrap(),
    ]);
    assert_eq!(rendered["documentedOperations"], 1);
    assert_eq!(rendered["updateFailures"], json!({}));
    let bundle = rendered["bundle"].as_str().unwrap();
    let data = read(f.bundle(bundle, "services/orders.json"));
    let rows = &data["operationExternalCalls"][&operation_id];
    assert_eq!(rows.as_array().unwrap().len(), 1);
    assert_eq!(rows[0]["eventId"], event_id);
    assert_eq!(rows[0]["contract"]["id"], outgoing.id);
    assert_ne!(rows[0]["contract"]["id"], incoming.id);
    assert_eq!(rows[0]["contractStatus"], "EXPLICIT_SAVED_CONTRACT");
    assert!(
        outgoing
            .source_ids
            .iter()
            .all(|id| data["operationSources"][&operation_id].get(id).is_some())
    );
    if let Some(output) = std::env::var_os("CODECLEW_EXTERNAL_CALL_READER_OUTPUT") {
        let output = std::path::PathBuf::from(output);
        fs::create_dir_all(&output).unwrap();
        for extension in ["html", "json"] {
            fs::copy(
                f.bundle(bundle, &format!("services/orders.{extension}")),
                output.join(format!("orders.{extension}")),
            )
            .unwrap();
        }
        fs::write(
            output.join("interaction.json"),
            serde_json::to_vec_pretty(&interaction).unwrap(),
        )
        .unwrap();
    }
    // Updating portable declarations recomposes retained source, never recaptures
    // the external destination or replaces the previous operation's details.
    interaction["addresses"][0]["url"] = json!("https://new-inventory.example.invalid/v2");
    interaction["contractReference"] = json!(incoming.id);
    put(&interaction);
    let (code, new) = f.run(&["docs", "recompose", "--snapshot", &original]);
    assert!(matches!(code, 0 | 3), "{new}");
    let retained = f.ok(&[
        "docs",
        "render",
        "--snapshot",
        new["snapshot"].as_str().unwrap(),
    ]);
    let retained = read(f.bundle(retained["bundle"].as_str().unwrap(), "services/orders.json"));
    let old = &retained["operationExternalCalls"][&operation_id][0];
    assert_eq!(
        old["addresses"][0]["url"],
        "https://inventory.example.invalid/v1"
    );
    assert_eq!(old["contract"]["id"], outgoing.id);
    assert_eq!(old["targetChanged"], true);
    assert_eq!(old["retained"], true);
    assert_eq!(read(f.bundle(bundle, "services/orders.json")), data);
    // Exact source-location selection does not admit an unverified cross-service
    // declared arrow or promote the incomplete external receiver to source proof.
    let mut wrong_arrow = narrative.clone();
    let wrong_event = wrong_arrow["operations"][0]["events"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|event| event["id"] == event_id)
        .unwrap();
    wrong_event["kind"] = json!("declared");
    wrong_event["interaction"] = json!("reserve-external");
    let wrong_arrow: clew::documentation::model::Narrative =
        serde_json::from_value(wrong_arrow).unwrap();
    assert!(
        clew::documentation::render::validate(&wrong_arrow, &checked)
            .unwrap_err()
            .message
            .contains("declared arrow endpoints or selected call site are unresolved")
    );
    // Unsupported executable schemes are rejected at the supported CLI boundary.
    let credential_address = ["https:", "//user:secret", "@example.invalid/"].concat();
    for url in [
        "javascript:alert(1)",
        "data:text/html,script",
        credential_address.as_str(),
    ] {
        let mut unsafe_value = interaction.clone();
        unsafe_value["addresses"][0]["url"] = json!(url);
        let input = f.input("unsafe-interaction.json", &unsafe_value);
        let digest = f.ok(&["docs", "interaction", "list"])["inputDigest"]
            .as_str()
            .unwrap()
            .to_owned();
        assert_ne!(
            f.run(&[
                "docs",
                "interaction",
                "put",
                "--input",
                input.to_str().unwrap(),
                "--expected-input-digest",
                &digest
            ])
            .0,
            0
        );
    }
    interaction.as_object_mut().unwrap().remove("addresses");
    interaction["contractReference"] = json!("unavailable-contract-id");
    put(&interaction);
    let mut competing = interaction.clone();
    competing["id"] = json!("competing-binding");
    competing["to"]["service"] = json!("other-external-service");
    competing
        .as_object_mut()
        .unwrap()
        .remove("contractReference");
    put(&competing);
    let (code, new) = f.run(&["docs", "recompose", "--snapshot", &original]);
    assert!(matches!(code, 0 | 3), "{new}");
    narrative["contextDigest"] = new["contextDigest"].clone();
    fs::write(&path, serde_json::to_vec(&narrative).unwrap()).unwrap();
    let updated = f.ok(&[
        "docs",
        "render",
        "--snapshot",
        new["snapshot"].as_str().unwrap(),
        "--input",
        path.to_str().unwrap(),
    ]);
    let data = read(f.bundle(updated["bundle"].as_str().unwrap(), "services/orders.json"));
    let rows = data["operationExternalCalls"][&operation_id]
        .as_array()
        .unwrap();
    assert_eq!(rows.len(), 2);
    assert!(rows.iter().all(
        |row| row["bindingStatus"] == "MULTIPLE_DECLARED_INTERACTIONS"
            && row["contract"].is_null()
            && row["addresses"] == json!([])
    ));
    assert!(
        rows.iter()
            .any(|row| row["contractStatus"] == "REFERENCE_UNAVAILABLE")
    );
    assert!(rows.iter().any(|row| row["contractStatus"] == "NOT_BOUND"));
}
