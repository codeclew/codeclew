//! Typed diagrams and decisions use the same retained publication lifecycle as prose.
#![cfg(unix)]
#[path = "support/documentation.rs"]
mod support;
use clew::documentation::{bindings, check::Check, store::Repository, work};
use serde_json::json;
use std::fs;
use support::{Fixture, read};

#[test]
fn section_visuals_publish_with_evidence_and_replay_without_source() {
    let f = Fixture::new();
    let source = f.service("orders");
    let checked = f.checked();
    let repo = Repository::open(&f.docs).unwrap();
    let snapshot = checked.save_snapshot(&repo).unwrap();
    let original_input = f.author("orders", &checked);
    let original_publication = f.ok(&[
        "docs",
        "render",
        "--input",
        original_input.to_str().unwrap(),
    ]);
    let original_data = read(f.bundle(
        original_publication["bundle"].as_str().unwrap(),
        "services/orders.json",
    ));
    let request = f.input("visual-work.json", &json!({"schema":"codeclew-documentation-work-request/1.0","audience":"Maintainers","entrypoint":"section-responsibilities","maxItems":100,"maxBytes":49152}));
    let mut page = f.ok(&[
        "docs",
        "work",
        "prepare",
        "--subject",
        "service:orders",
        "--snapshot",
        &snapshot,
        "--input",
        request.to_str().unwrap(),
    ]);
    let id = page["work"].as_str().unwrap().to_owned();
    while let Some(cursor) = page["nextCursor"].as_str() {
        let selection = f.input("visual-page.json", &json!({"cursor":cursor}));
        page = f.ok(&[
            "docs",
            "work",
            "read",
            "--work",
            &id,
            "--input",
            selection.to_str().unwrap(),
        ]);
    }
    let frozen = work::load(&repo, &id).unwrap();
    let section = frozen
        .handles
        .iter()
        .find(|(_, h)| h.id == "section-responsibilities")
        .unwrap()
        .0;
    let reserve = checked.services["orders"]
        .entrypoints
        .iter()
        .find(|e| e.symbol.contains("reserve"))
        .unwrap();
    let normalize = checked.services["orders"]
        .entrypoints
        .iter()
        .find(|e| e.symbol.contains("normalize"))
        .unwrap();
    let entry = frozen
        .handles
        .iter()
        .find(|(_, h)| h.kind == "ENTRYPOINT" && h.id == reserve.id)
        .unwrap()
        .0;
    let helper = frozen
        .handles
        .iter()
        .find(|(_, h)| h.kind == "ENTRYPOINT" && h.id == normalize.id)
        .unwrap()
        .0;
    for reference in [entry, helper] {
        let selection = f.input("visual-evidence.json", &json!({"references":[reference]}));
        f.ok(&[
            "docs",
            "work",
            "read",
            "--work",
            &id,
            "--input",
            selection.to_str().unwrap(),
        ]);
    }
    let claim = |text: &str| json!({"text":text,"evidence":[entry]});
    let mut graph = json!({"id":"dispatch","kind":"execution-flow","title":"Quantity dispatch","purpose":claim("Show the selected quantity path."),"scope":claim("The selected method and local normalization helper."),"limitations":["Static source interpretation; no runtime trace."],"nodes":[{"id":"receive","meaning":claim("Receive quantity.")},{"id":"normalize","meaning":claim("Normalize quantity.")}],"edges":[{"id":"call","from":"receive","to":"normalize","meaning":claim("Call the normalization helper.")} ]});
    graph["nodes"][1]["meaning"]["evidence"] = json!([helper]);
    let table = json!({"id":"normalization","kind":"decision-table","title":"Local normalization","purpose":claim("Record the selected normalization result."),"scope":claim("Applies at the helper call."),"limitations":["Illustrative source interpretation, not executable DMN."],"parent":{"artifact":"dispatch","node":"normalize"},"hitPolicy":"FIRST","policyExplanation":claim("A single applicable row supplies the value."),"rules":[{"condition":claim("For any supplied quantity."),"outcome":claim("Return the supplied quantity.")}],"afterSelection":claim("No provider delivery is established.")});
    let proposal = json!({"schema":"codeclew-documentation-proposal/1.0","operations":[{"entrypoint":section,"title":"Responsibilities","summary":claim("Normalizes the requested quantity."),"steps":[],"visuals":[graph,table]}]});
    let mut forged = proposal.clone();
    forged["operations"][0]["visuals"][0]["edges"][0]["meaning"]["evidence"] =
        json!(["unread-evidence"]);
    let input = f.input("visual-forged.json", &forged);
    let (_, rejection) = f.run(&[
        "docs",
        "proposal",
        "submit",
        "--work",
        &id,
        "--input",
        input.to_str().unwrap(),
    ]);
    assert!(
        rejection.to_string().contains("unknown work reference"),
        "{rejection}"
    );
    let input = f.input("visual-proposal.json", &proposal);
    let submitted = f.ok(&[
        "docs",
        "proposal",
        "submit",
        "--work",
        &id,
        "--input",
        input.to_str().unwrap(),
    ]);
    assert_eq!(submitted["status"], "READY_WITH_LIMITATIONS", "{submitted}");
    let published = f.ok(&[
        "docs",
        "proposal",
        "publish",
        "--proposal",
        submitted["proposal"].as_str().unwrap(),
        "--unassessed",
    ]);
    assert_eq!(published["updateFailures"], json!({}));
    let bundle = published["bundle"].as_str().unwrap();
    let data = read(f.bundle(bundle, "services/orders.json"));
    for original in original_data["operations"].as_array().unwrap() {
        let retained = data["operations"]
            .as_array()
            .unwrap()
            .iter()
            .find(|operation| operation["id"] == original["id"])
            .unwrap();
        assert_eq!(
            retained, original,
            "section-only publication changed an existing operation"
        );
    }
    let op = data["operations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|o| o["id"] == "section-responsibilities")
        .unwrap();
    assert_eq!(op["visuals"].as_array().unwrap().len(), 2);
    assert_eq!(op["visuals"][1]["parent"]["node"], "normalize");
    let visual_only_sources: Vec<_> = op["visuals"][0]["nodes"][1]["meaning"]["sourceIds"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|sid| !op["summary"]["sourceIds"].as_array().unwrap().contains(sid))
        .collect();
    assert!(
        !visual_only_sources.is_empty(),
        "helper evidence must be independent of summary evidence"
    );
    for source in visual_only_sources {
        assert!(
            data["sources"].get(source.as_str().unwrap()).is_some(),
            "visual-only source omitted"
        );
    }
    for visual in op["visuals"].as_array().unwrap() {
        for sid in visual["purpose"]["sourceIds"].as_array().unwrap() {
            assert!(data["sources"].get(sid.as_str().unwrap()).is_some());
        }
    }
    let binding = bindings::baseline(&repo).unwrap().unwrap().1;
    let key = "service:orders/section-responsibilities/visual-dispatch";
    assert!(!binding.fragments[key].dependencies.is_empty());
    assert!(!binding.fragments[key].sources.is_empty());
    assert_eq!(
        binding.fragments[key].content["edges"][0]["from"],
        "receive"
    );
    let accepted = binding.accepted_versions["service:orders/section-responsibilities"].clone();
    // Construct a historical-renderer fixture without changing its evidence,
    // accepted content or output hashes. Production baselines are never edited.
    let binding_path = f.bundle(bundle, "bindings.json");
    let mut historical = read(binding_path.clone());
    historical["renderer"] = json!("codeclew-documentation-html/1.14");
    fs::write(&binding_path, serde_json::to_vec(&historical).unwrap()).unwrap();
    let preserved_bytes = fs::read(&binding_path).unwrap();
    let historical_baseline = bindings::baseline(&repo).unwrap().unwrap().1;
    assert_eq!(
        historical_baseline.renderer,
        "codeclew-documentation-html/1.14"
    );
    assert_eq!(
        serde_json::to_value(&historical_baseline.accepted_versions).unwrap(),
        serde_json::to_value(&binding.accepted_versions).unwrap()
    );
    fs::remove_dir_all(source).unwrap();
    let (code, recomposed) = f.run(&["docs", "recompose", "--snapshot", &snapshot]);
    assert!(matches!(code, 0 | 3), "{recomposed}");
    let replay = f.ok(&[
        "docs",
        "render",
        "--snapshot",
        recomposed["snapshot"].as_str().unwrap(),
    ]);
    let replay_data = read(f.bundle(replay["bundle"].as_str().unwrap(), "services/orders.json"));
    let replay_op = replay_data["operations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|o| o["id"] == "section-responsibilities")
        .unwrap();
    assert_eq!(op["visuals"], replay_op["visuals"]);
    let rebound = bindings::baseline(&repo).unwrap().unwrap().1;
    assert_eq!(rebound.renderer, clew::documentation::model::RENDERER);
    assert_eq!(
        replay_data["renderer"],
        clew::documentation::model::RENDERER
    );
    assert_eq!(fs::read(&binding_path).unwrap(), preserved_bytes);
    assert_eq!(rebound.retained_sources, binding.retained_sources);
    assert_eq!(
        serde_json::to_value(accepted).unwrap(),
        serde_json::to_value(&rebound.accepted_versions["service:orders/section-responsibilities"])
            .unwrap()
    );
    assert!(Check::load_snapshot(&repo, &snapshot).is_ok());

    // A later summary edit cannot erase retained diagrams by omission. The
    // author must supply their complete replacement set or explicitly remove it.
    let mut next_page = f.ok(&[
        "docs",
        "work",
        "prepare",
        "--subject",
        "service:orders",
        "--snapshot",
        recomposed["snapshot"].as_str().unwrap(),
        "--input",
        request.to_str().unwrap(),
    ]);
    let next_id = next_page["work"].as_str().unwrap().to_owned();
    while let Some(cursor) = next_page["nextCursor"].as_str() {
        let selection = f.input("next-visual-page.json", &json!({"cursor":cursor}));
        next_page = f.ok(&[
            "docs",
            "work",
            "read",
            "--work",
            &next_id,
            "--input",
            selection.to_str().unwrap(),
        ]);
    }
    let next_work = work::load(&repo, &next_id).unwrap();
    let next_entry = next_work
        .handles
        .iter()
        .find(|(_, h)| h.kind == "ENTRYPOINT" && h.id == reserve.id)
        .unwrap()
        .0;
    let next_section = next_work
        .handles
        .iter()
        .find(|(_, h)| h.kind == "SECTION" && h.id == "section-responsibilities")
        .unwrap()
        .0;
    let selection = f.input(
        "next-visual-evidence.json",
        &json!({"references":[next_entry]}),
    );
    f.ok(&[
        "docs",
        "work",
        "read",
        "--work",
        &next_id,
        "--input",
        selection.to_str().unwrap(),
    ]);
    let mut summary_only = json!({"schema":"codeclew-documentation-proposal/1.0","operations":[{
        "entrypoint":next_section,"title":"Responsibilities", "steps":[],
        "summary":{"text":"Normalizes the requested quantity.","evidence":[next_entry]}
    }]});
    let input = f.input("summary-without-visual-decision.json", &summary_only);
    let rejected = f.ok(&[
        "docs",
        "proposal",
        "submit",
        "--work",
        &next_id,
        "--input",
        input.to_str().unwrap(),
    ]);
    assert_eq!(rejected["status"], "NEEDS_REPAIR", "{rejected}");
    assert!(
        rejected.to_string().contains("explicit visuals array"),
        "{rejected}"
    );
    assert_eq!(
        bindings::baseline(&repo).unwrap().unwrap().0,
        replay["bundle"].as_str().unwrap()
    );
    summary_only["operations"][0]["visuals"] = json!([]);
    let input = f.input("summary-explicit-visual-removal.json", &summary_only);
    let removal = f.ok(&[
        "docs",
        "proposal",
        "submit",
        "--work",
        &next_id,
        "--input",
        input.to_str().unwrap(),
    ]);
    assert!(
        removal["status"].as_str().unwrap().starts_with("READY_"),
        "{removal}"
    );
    let removed = f.ok(&[
        "docs",
        "proposal",
        "publish",
        "--proposal",
        removal["proposal"].as_str().unwrap(),
        "--unassessed",
    ]);
    let removed_data = read(f.bundle(removed["bundle"].as_str().unwrap(), "services/orders.json"));
    let removed_operation = removed_data["operations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|op| op["id"] == "section-responsibilities")
        .unwrap();
    assert!(removed_operation.get("visuals").is_none());
    assert_eq!(
        read(f.bundle(replay["bundle"].as_str().unwrap(), "services/orders.json")),
        replay_data
    );
}
