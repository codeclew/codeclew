//! A selected check must not reacquire unchanged siblings or erase their evidence.
#![cfg(unix)]
#[path = "support/documentation.rs"]
mod support;
use clew::documentation::{check::Check, store::Repository};
use serde_json::{Value, json};
use std::fs;
use support::Fixture;

#[test]
fn scoped_check_preserves_offline_siblings_across_selected_catalog_change() {
    let f = Fixture::new();
    let orders = f.service("orders");
    f.service("inventory");
    let shipping = f.service("shipping");
    let initial = f.checked();
    let repo = Repository::open(&f.docs).unwrap();
    let original = initial.save_snapshot(&repo).unwrap();
    // Neither source can be revisited. Even doctor/admission would fail.
    fs::rename(&orders, orders.with_extension("offline")).unwrap();
    fs::rename(&shipping, shipping.with_extension("offline")).unwrap();
    let path = f.docs.join("catalog/services/inventory.json");
    let mut service: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    service["title"] = json!("Updated inventory documentation");
    fs::write(&path, serde_json::to_vec(&service).unwrap()).unwrap();

    let (_, report) = f.run(&["docs", "check", "--service", "inventory"]);
    assert_eq!(report["status"], "CHECKED", "{report}");
    let checked = Check::load_snapshot(&repo, report["snapshot"].as_str().unwrap()).unwrap();
    assert_eq!(checked.services["orders"], initial.services["orders"]);
    assert_eq!(checked.services["shipping"], initial.services["shipping"]);
    let inputs = checked.source_inputs.as_ref().unwrap();
    assert_eq!(
        inputs.selected_services.iter().collect::<Vec<_>>(),
        vec!["inventory"]
    );
    assert_eq!(
        inputs.retained_services.iter().collect::<Vec<_>>(),
        vec!["orders", "shipping"]
    );
    assert_eq!(
        report["services"]["orders"]["sourceAuthority"],
        "RETAINED_SOURCE_NOT_REVERIFIED"
    );
    assert_eq!(
        report["services"]["inventory"]["sourceAuthority"],
        "CAPTURED_SOURCE"
    );
    let (code, context) = f.run(&["docs", "context", "--service", "orders"]);
    assert_eq!(code, 0, "{context}");
    assert_eq!(
        context["sourceAuthorities"]["orders"],
        "RETAINED_SOURCE_NOT_REVERIFIED"
    );
    let (code, rendered) = f.run(&["docs", "render"]);
    assert!(matches!(code, 0 | 4), "{rendered}");
    assert_eq!(
        Check::load_snapshot(&repo, &original).unwrap().services["orders"],
        initial.services["orders"]
    );
}

#[test]
fn selected_failure_and_changed_sibling_are_not_hidden_by_saved_evidence() {
    let f = Fixture::new();
    let orders = f.service("orders");
    let inventory = f.service("inventory");
    f.checked();
    fs::rename(&inventory, inventory.with_extension("offline")).unwrap();
    let path = f.docs.join("catalog/services/orders.json");
    let mut service: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    service["source"]["roots"] = json!(["different-source-scope"]);
    fs::write(&path, serde_json::to_vec(&service).unwrap()).unwrap();
    fs::rename(&orders, orders.with_extension("offline")).unwrap();
    let (_, report) = f.run(&["docs", "check", "--service", "inventory"]);
    assert_eq!(report["status"], "UNRESOLVED", "{report}");
    assert_eq!(
        report["unresolved"]["orders"]["reason"],
        "SERVICE_NOT_SELECTED"
    );
    assert_eq!(report["unresolved"]["inventory"]["status"], "UNRESOLVED");
    let repo = Repository::open(&f.docs).unwrap();
    let checked = Check::load_snapshot(&repo, report["snapshot"].as_str().unwrap()).unwrap();
    assert!(checked.services.is_empty());
    assert!(checked.source_inputs.unwrap().retained_services.is_empty());
}

#[test]
fn concurrent_selected_checks_keep_both_new_service_versions() {
    let f = Fixture::new();
    let mut other_runtime = Fixture::new();
    // Separate runtime/state roots must still serialize on the shared docs root.
    other_runtime.docs = f.docs.clone();
    let orders = f.service("orders");
    let inventory = f.service("inventory");
    let initial = f.checked();
    for (path, number) in [(&orders, 2), (&inventory, 3)] {
        fs::write(path.join("Orders.java"), format!("public class Orders {{ public int reserve(int quantity) {{ return quantity + {number}; }} }}\n")).unwrap();
        support::commit(path);
    }
    let barrier = std::sync::Barrier::new(2);
    std::thread::scope(|scope| {
        let tasks = [("orders", &f), ("inventory", &other_runtime)].map(|(service, f)| {
            let barrier = &barrier;
            scope.spawn(move || {
                barrier.wait();
                let (_, report) = f.run(&["docs", "check", "--service", service]);
                assert_eq!(report["status"], "CHECKED", "{report}");
                report
            })
        });
        let reports = tasks.map(|task| task.join().unwrap());
        let repo = Repository::open(&f.docs).unwrap();
        let (latest, _) = Check::retained(&repo, None, &Default::default()).unwrap();
        for (service, report) in ["orders", "inventory"].into_iter().zip(reports) {
            assert_ne!(
                latest.services[service].revision,
                initial.services[service].revision
            );
            assert_eq!(
                latest.services[service].revision,
                report["services"][service]["revision"].as_str().unwrap()
            );
        }
    });
}
