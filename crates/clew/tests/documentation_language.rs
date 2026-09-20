//! Author language is independent from retained source evidence and immutable history.
#![cfg(unix)]
#[path = "support/documentation.rs"]
mod support;
use clew::documentation::{bindings, check::Check, store::Repository, work};
use serde_json::{Value, json};
use std::{collections::BTreeMap, fs, path::Path};
use support::{Fixture, read};

fn files(path: &Path) -> BTreeMap<std::path::PathBuf, Vec<u8>> {
    let mut out = BTreeMap::new();
    for entry in fs::read_dir(path).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            out.extend(files(&path));
        } else {
            out.insert(path.clone(), fs::read(path).unwrap());
        }
    }
    out
}

fn prepare(f: &Fixture, snapshot: &str, entrypoint: &str, language: Option<&str>) -> work::Work {
    let request = f.input(
        "language-work.json",
        &json!({
            "schema":"codeclew-documentation-work-request/1.0","audience":"Maintainers",
            "entrypoint":entrypoint,"maxItems":100,"maxBytes":49152
        }),
    );
    let mut args = vec![
        "docs",
        "work",
        "prepare",
        "--subject",
        "service:orders",
        "--snapshot",
        snapshot,
        "--input",
        request.to_str().unwrap(),
    ];
    if let Some(language) = language {
        args.extend(["--language", language]);
    }
    let mut page = f.ok(&args);
    let id = page["work"].as_str().unwrap().to_owned();
    while let Some(cursor) = page["nextCursor"].as_str() {
        let selection = f.input("language-page.json", &json!({"cursor":cursor}));
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
    work::load(&Repository::open(&f.docs).unwrap(), &id).unwrap()
}

fn author(f: &Fixture, work: &work::Work, entrypoint: &str, title: &str, text: &str) -> Value {
    let reference = work
        .handles
        .iter()
        .find(|(_, h)| h.kind == "ENTRYPOINT" && h.id == entrypoint)
        .unwrap()
        .0;
    let selection = f.input("language-evidence.json", &json!({"references":[reference]}));
    f.ok(&[
        "docs",
        "work",
        "read",
        "--work",
        &work.id,
        "--input",
        selection.to_str().unwrap(),
    ]);
    let symbol = &work.checked.services["orders"]
        .entrypoints
        .iter()
        .find(|entry| entry.id == entrypoint)
        .unwrap()
        .symbol;
    let flows = work
        .handles
        .iter()
        .filter(|(_, handle)| {
            handle.kind == "DEPENDENCY"
                && work
                    .checked
                    .dependencies
                    .get(&handle.id)
                    .is_some_and(|dependency| {
                        dependency.kind == "FLOW" && &dependency.symbol == symbol
                    })
        })
        .map(|(reference, _)| reference.clone())
        .collect::<Vec<_>>();
    assert!(!flows.is_empty());
    for batch in flows.chunks(8) {
        let selection = f.input("language-flow-evidence.json", &json!({"references":batch}));
        f.ok(&[
            "docs",
            "work",
            "read",
            "--work",
            &work.id,
            "--input",
            selection.to_str().unwrap(),
        ]);
    }
    let steps = flows
        .iter()
        .map(|reference| json!({"kind":"note","meaning":{"text":text,"evidence":[reference]}}))
        .collect::<Vec<_>>();
    let proposal = f.input(
        "language-proposal.json",
        &json!({
            "schema":"codeclew-documentation-proposal/1.0","operations":[{
                "entrypoint":reference,"title":title,"summary":{"text":text,"evidence":[reference]},
                "steps":steps
            }]
        }),
    );
    let submitted = f.ok(&[
        "docs",
        "proposal",
        "submit",
        "--work",
        &work.id,
        "--input",
        proposal.to_str().unwrap(),
    ]);
    assert!(
        submitted["status"].as_str().unwrap().starts_with("READY_"),
        "{submitted}"
    );
    f.ok(&[
        "docs",
        "proposal",
        "publish",
        "--proposal",
        submitted["proposal"].as_str().unwrap(),
        "--unassessed",
    ])
}

fn operation<'a>(data: &'a Value, id: &str) -> &'a Value {
    data["operations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|op| op["id"] == id)
        .unwrap()
}

fn fallback(f: &Fixture, publication: &Value, data: &Value, id: &str, expected_bundle: &str) {
    let href = data["translationGaps"][id]["href"]
        .as_str()
        .expect("translation gap needs a history fallback");
    assert!(href.contains(expected_bundle), "{href}");
    let (path, fragment) = href.split_once('#').unwrap();
    assert_eq!(fragment, id);
    let page = f.bundle(
        publication["bundle"].as_str().unwrap(),
        "services/orders.html",
    );
    assert!(
        page.parent().unwrap().join(path).is_file(),
        "broken history link: {href}"
    );
}

#[test]
fn language_changes_authoring_without_recapture_or_relabeling_history() {
    let f = Fixture::new();
    let checkout = f.service("orders");
    let checked = f.checked();
    let repo = Repository::open(&f.docs).unwrap();
    let snapshot = checked.save_snapshot(&repo).unwrap();
    let captured = files(&f.docs.join(".codeclew/cache"))
        .into_iter()
        .filter(|(path, _)| {
            path.extension()
                .is_some_and(|extension| extension == "json")
        })
        .collect::<BTreeMap<_, _>>();
    let captured_evidence =
        serde_json::to_value(Check::load_snapshot(&repo, &snapshot).unwrap()).unwrap();
    let entrypoint = checked.services["orders"]
        .entrypoints
        .iter()
        .find(|e| e.symbol.contains("reserve"))
        .unwrap()
        .id
        .clone();

    // The ordinary historical fixture has no author-language metadata. Never infer English.
    let input = f.author("orders", &checked);
    let original = f.ok(&[
        "docs",
        "render",
        "--snapshot",
        &snapshot,
        "--input",
        input.to_str().unwrap(),
    ]);
    let original_bundle = original["bundle"].as_str().unwrap();
    let original_files = files(&f.bundle(original_bundle, ""));
    let original_data = read(f.bundle(original_bundle, "services/orders.json"));
    assert!(
        operation(&original_data, &entrypoint)
            .get("documentationLanguage")
            .is_none()
    );
    fs::remove_dir_all(checkout).unwrap();

    let unknown_ru = f.ok(&[
        "docs",
        "render",
        "--snapshot",
        &snapshot,
        "--language",
        "ru",
    ]);
    let unknown_data = read(f.bundle(
        unknown_ru["bundle"].as_str().unwrap(),
        "services/orders.json",
    ));
    assert_eq!(
        unknown_data["translationGaps"][&entrypoint]["availableLanguage"],
        Value::Null
    );
    assert_eq!(
        operation(&unknown_data, &entrypoint),
        operation(&original_data, &entrypoint)
    );
    fallback(&f, &unknown_ru, &unknown_data, &entrypoint, original_bundle);
    assert_eq!(unknown_ru["documentedOperations"], 0);

    let english_work = prepare(&f, &snapshot, &entrypoint, Some("en"));
    let english = author(
        &f,
        &english_work,
        &entrypoint,
        "Reserve quantity",
        "Returns the normalized requested quantity.",
    );
    let english_bundle = english["bundle"].as_str().unwrap();
    let english_files = files(&f.bundle(english_bundle, ""));
    let english_data = read(f.bundle(english_bundle, "services/orders.json"));
    assert_eq!(english["documentationLanguage"], "en");
    assert_eq!(english["documentedOperations"], 1);
    assert_eq!(
        operation(&english_data, &entrypoint)["documentationLanguage"],
        "en"
    );
    let english_version = bindings::baseline(&repo)
        .unwrap()
        .unwrap()
        .1
        .accepted_versions[&format!("service:orders/{entrypoint}")]
        .clone();
    assert_eq!(
        english_version
            .external_request
            .documentation_language
            .as_deref(),
        Some("en")
    );

    let russian_view = f.ok(&[
        "docs",
        "render",
        "--snapshot",
        &snapshot,
        "--language",
        "ru",
    ]);
    let russian_data = read(f.bundle(
        russian_view["bundle"].as_str().unwrap(),
        "services/orders.json",
    ));
    assert_eq!(russian_data["requestedDocumentationLanguage"], "ru");
    assert_eq!(russian_data["translationComplete"], false);
    assert_eq!(russian_view["documentedOperations"], 0);
    assert_eq!(russian_view["documentedSections"], 0);
    assert_eq!(
        operation(&russian_data, &entrypoint),
        operation(&english_data, &entrypoint)
    );
    assert_eq!(russian_data["sources"], english_data["sources"]);
    fallback(
        &f,
        &russian_view,
        &russian_data,
        &entrypoint,
        english_bundle,
    );
    let preserved = bindings::baseline(&repo).unwrap().unwrap().1;
    assert_eq!(
        serde_json::to_value(&preserved.accepted_versions[&format!("service:orders/{entrypoint}")])
            .unwrap(),
        serde_json::to_value(&english_version).unwrap()
    );

    // Omission inherits the active publication language, while an explicit choice overrides it.
    let russian_work = prepare(&f, &snapshot, &entrypoint, None);
    assert_eq!(russian_work.request.documentation_language(), "ru");
    assert_ne!(russian_work.id, english_work.id);
    assert_eq!(russian_work.snapshot, english_work.snapshot);
    assert_eq!(
        russian_work.checked.context_digest,
        english_work.checked.context_digest
    );
    assert_eq!(russian_work.influence, english_work.influence);
    let override_work = prepare(&f, &snapshot, &entrypoint, Some("en"));
    assert_eq!(override_work.request.documentation_language(), "en");
    assert_ne!(override_work.id, russian_work.id);
    let russian = author(
        &f,
        &russian_work,
        &entrypoint,
        "Запрос количества",
        "Возвращает нормализованное запрошенное количество.",
    );
    assert_eq!(russian["documentationLanguage"], "ru");
    assert_eq!(russian["documentedOperations"], 1);
    assert_eq!(russian["translationComplete"], true);
    let accepted_data = read(f.bundle(russian["bundle"].as_str().unwrap(), "services/orders.json"));
    assert_eq!(
        operation(&accepted_data, &entrypoint)["documentationLanguage"],
        "ru"
    );
    assert_eq!(
        operation(&accepted_data, &entrypoint)["summary"]["sourceIds"],
        operation(&english_data, &entrypoint)["summary"]["sourceIds"]
    );
    assert_eq!(accepted_data["sources"], english_data["sources"]);
    let repeated = f.ok(&[
        "docs",
        "render",
        "--snapshot",
        &snapshot,
        "--language",
        "ru",
    ]);
    assert_eq!(repeated["bundle"], russian["bundle"]);
    assert_eq!(files(&f.bundle(original_bundle, "")), original_files);
    assert_eq!(files(&f.bundle(english_bundle, "")), english_files);
    for (path, bytes) in captured {
        assert_eq!(
            fs::read(&path).unwrap(),
            bytes,
            "capture changed: {}",
            path.display()
        );
    }
    assert_eq!(
        serde_json::to_value(Check::load_snapshot(&repo, &snapshot).unwrap()).unwrap(),
        captured_evidence
    );
}
