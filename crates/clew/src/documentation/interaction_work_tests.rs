//! Declaration evidence must have identical scope through every Work read path.
use super::{digest, model::Observation, store::Repository, work};
use serde_json::json;
use std::collections::BTreeSet;

#[test]
fn interaction_work_query_explicit_and_parts_share_service_scope() {
    let mut work = work::api_contract_tests::endpoint_context_fixture();
    work.request.context_profile = None;
    for (id, kind, service, from, to) in [
        ("outgoing", "DECLARED_INTERACTION", "", "orders", "external"),
        ("incoming", "DECLARED_INTERACTION", "", "external", "orders"),
        (
            "unrelated",
            "DECLARED_INTERACTION",
            "",
            "inventory",
            "external",
        ),
        ("foreign-code", "SYMBOL", "inventory", "orders", "external"),
    ] {
        let id = format!("interaction:{id}");
        let normalized = json!({"from":{"service":from},"to":{"service":to},"external":true});
        let record = Observation {
            id: id.clone(),
            kind: kind.into(),
            service: service.into(),
            symbol: id.clone(),
            digest: digest(&normalized).unwrap(),
            normalized,
            source_ids: vec![],
        };
        work.influence.insert(id.clone(), record.digest.clone());
        work.handles.insert(
            id.clone(),
            work::Handle {
                kind: "DEPENDENCY".into(),
                id: id.clone(),
            },
        );
        work.checked.dependencies.insert(id, record);
    }
    for kind in ["DECLARED_INTERACTION", "*"] {
        let rows = work::rows(
            &work,
            &serde_json::from_value(json!({
                "query":{"kind":kind,"symbolContains":"interaction:"}
            }))
            .unwrap(),
        )
        .unwrap();
        assert_eq!(
            rows.iter()
                .map(|r| r["id"].as_str().unwrap())
                .collect::<BTreeSet<_>>(),
            BTreeSet::from(["interaction:incoming", "interaction:outgoing"])
        );
    }
    // Explicit reads must not confer authority over a declaration's other service.
    for id in ["outgoing", "incoming", "unrelated", "foreign-code"] {
        let selection =
            serde_json::from_value(json!({"references":[format!("interaction:{id}")]})).unwrap();
        let result = work::rows(&work, &selection);
        if matches!(id, "outgoing" | "incoming") {
            assert!(
                result
                    .unwrap()
                    .iter()
                    .any(|row| row["id"] == format!("interaction:{id}"))
            );
        } else {
            assert!(
                result
                    .unwrap_err()
                    .message
                    .contains("unknown service dependency")
            );
        }
    }
    let temp = tempfile::tempdir().unwrap();
    Repository::init(temp.path(), "Interaction evidence scope").unwrap();
    let repo = Repository::open(temp.path()).unwrap();
    work::api_contract_tests::persist_operation_fixture(&repo, &mut work);
    for id in ["outgoing", "incoming", "unrelated", "foreign-code"] {
        let result = work::read_dependency_part(
            &repo,
            &work.id,
            serde_json::from_value(json!({
                "schema":"codeclew-documentation-dependency-part-request/1.0",
                "kind":"DEPENDENCY","id":format!("interaction:{id}")
            }))
            .unwrap(),
        );
        if matches!(id, "outgoing" | "incoming") {
            assert!(result.is_ok(), "{id}: {result:?}");
        } else {
            assert!(
                result
                    .unwrap_err()
                    .message
                    .contains("unknown service dependency")
            );
        }
    }
}
