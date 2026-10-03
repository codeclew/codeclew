//! Public CLI regression for an oversized authored operation and a bounded edit.
//! Read/proposal readiness is deliberately separate from reviewed publication.
#![cfg(unix)]

#[path = "support/documentation.rs"]
mod support;

use clew::{
    canonical,
    documentation::{check::Check, model::Operation, proposals, store::Repository, work},
};
use serde_json::{Value, json};
use std::{collections::BTreeMap, fs, path::Path, process::Output};
use support::{Fixture, read};

const MAX_BYTES: usize = 49_152;
const REQUEST_SCHEMA: &str = "codeclew-documentation-retained-part-request/1.0";

fn bundle_files(root: &Path) -> BTreeMap<String, Vec<u8>> {
    fn visit(root: &Path, directory: &Path, result: &mut BTreeMap<String, Vec<u8>>) {
        for entry in fs::read_dir(directory).unwrap() {
            let entry = entry.unwrap();
            if entry.file_type().unwrap().is_dir() {
                visit(root, &entry.path(), result);
            } else {
                assert!(entry.file_type().unwrap().is_file());
                result.insert(
                    entry
                        .path()
                        .strip_prefix(root)
                        .unwrap()
                        .to_str()
                        .unwrap()
                        .into(),
                    fs::read(entry.path()).unwrap(),
                );
            }
        }
    }
    let mut result = BTreeMap::new();
    visit(root, root, &mut result);
    result
}

fn successful_json(output: Output) -> Value {
    assert!(
        output.status.success(),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    assert_eq!(output.stdout.last(), Some(&b'\n'));
    assert!(
        output.stdout.len() <= MAX_BYTES,
        "{} response bytes",
        output.stdout.len()
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

fn prepare(f: &Fixture, snapshot: &str, entrypoint: &str, audience: &str) -> String {
    let input = f.input(
        "retained-work-request.json",
        &json!({
            "schema":"codeclew-documentation-work-request/1.0", "audience":audience,
            "entrypoint":entrypoint, "maxItems":100, "maxBytes":MAX_BYTES,
        }),
    );
    f.ok(&[
        "docs",
        "work",
        "prepare",
        "--subject",
        "service:orders",
        "--snapshot",
        snapshot,
        "--input",
        input.to_str().unwrap(),
    ])["work"]
        .as_str()
        .unwrap()
        .to_owned()
}

fn read_pages(f: &Fixture, id: &str) -> Vec<Value> {
    let mut cursor = None;
    let mut pages = Vec::new();
    loop {
        let selection = cursor
            .as_ref()
            .map_or_else(|| json!({}), |cursor| json!({"cursor":cursor}));
        let input = f.input("retained-work-selection.json", &selection);
        let page = successful_json(f.run_raw(&[
            "docs",
            "work",
            "read",
            "--work",
            id,
            "--input",
            input.to_str().unwrap(),
        ]));
        let next = page["nextCursor"].as_str().map(str::to_owned);
        assert!(
            next.is_none() || next != cursor,
            "page cursor must progress"
        );
        cursor = next;
        pages.push(page);
        if cursor.is_none() {
            return pages;
        }
    }
}

fn part(f: &Fixture, work: &str, id: &str, kind: &str, cursor: Option<&str>) -> Output {
    let mut request = json!({"schema":REQUEST_SCHEMA, "kind":kind, "id":id});
    if let Some(cursor) = cursor {
        request["cursor"] = json!(cursor);
    }
    let input = f.input("retained-part-request.json", &request);
    f.run_raw(&[
        "docs",
        "work",
        "read-retained-part",
        "--work",
        work,
        "--input",
        input.to_str().unwrap(),
    ])
}

fn submit(f: &Fixture, work: &str, proposal: &Value) -> Value {
    let input = f.input("retained-title-proposal.json", proposal);
    f.ok(&[
        "docs",
        "proposal",
        "submit",
        "--work",
        work,
        "--input",
        input.to_str().unwrap(),
    ])
}

#[test]
fn oversized_authored_operation_is_part_readable_and_title_edit_is_machine_ready_offline() {
    let f = Fixture::new();
    // This public source-syntax fixture uses the native parser, without a JVM or
    // compiler worker. The oversized record below is authored prose, not source.
    let source_repo = f.service("orders");
    let (code, checked_result) = f.run(&["docs", "check"]);
    assert!(matches!(code, 0 | 3 | 4), "{checked_result}");
    let snapshot = checked_result["snapshot"].as_str().unwrap().to_owned();
    let repo = Repository::open(&f.docs).unwrap();
    let checked = Check::load_snapshot(&repo, &snapshot).unwrap();
    let narrative_path = f.author("orders", &checked);
    let mut narrative = read(&narrative_path);
    let entrypoint = narrative["operations"][0]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let paragraph_template = narrative["operations"][0]["explanation"][0].clone();
    let prose = "Synthetic retained explanation λ🔥: the quantity is normalized. Quote \" and slash \\ remain literal.\n".repeat(60);
    assert!(prose.len() <= 8192);
    for index in 0..12 {
        let mut paragraph = paragraph_template.clone();
        paragraph["id"] = json!(format!("retained-paragraph-{index}"));
        paragraph["text"] = json!(format!(
            "Retained fixture paragraph {index}. {prose} END_終端_{index}"
        ));
        assert!(paragraph["text"].as_str().unwrap().len() <= 8192);
        paragraph["detail"] = json!(index % 2 == 0);
        narrative["operations"][0]["explanation"]
            .as_array_mut()
            .unwrap()
            .push(paragraph);
    }
    let authored: Operation = serde_json::from_value(narrative["operations"][0].clone()).unwrap();
    let expected = canonical::bytes(&authored).unwrap();
    assert!(expected.len() > MAX_BYTES);
    let input = f.input("oversized-authored-narrative.json", &narrative);
    // Seed an ordinary frozen publication through the supported narrative path.
    // This is not a claim of separate meaning review for the synthetic fixture.
    let publication = f.ok(&[
        "docs",
        "render",
        "--snapshot",
        &snapshot,
        "--input",
        input.to_str().unwrap(),
        "--publish",
    ]);
    assert_eq!(publication["updateFailures"], json!({}), "{publication}");
    let bundle = publication["bundle"].as_str().unwrap().to_owned();
    let baseline = read(f.bundle(&bundle, "bindings.json"));
    let retained = baseline["narratives"]["service:orders"]["operations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|operation| operation["id"] == entrypoint)
        .unwrap();
    let original: Operation = serde_json::from_value(retained.clone()).unwrap();
    assert_eq!(canonical::bytes(&original).unwrap(), expected);
    let frozen_bundle = bundle_files(&f.bundle(&bundle, ""));
    let index_before = fs::read(f.docs.join("docs/index.html")).unwrap();
    let latest_before = fs::read(f.docs.join(".codeclew/cache/latest-check.json")).unwrap();
    assert_eq!(
        f.ok(&["docs", "history", "show", "--id", &bundle])["status"],
        "FROZEN_SNAPSHOT"
    );

    let work_id = prepare(
        &f,
        &snapshot,
        &entrypoint,
        "Synthetic retained title correction",
    );
    let work_manifest = f.docs.join(format!(".codeclew/work/{work_id}/work.json"));
    let work_before = fs::read(&work_manifest).unwrap();
    // All subsequent reads, preparations and submission must reuse saved input.
    fs::rename(&source_repo, source_repo.with_extension("offline")).unwrap();
    let pages = read_pages(&f, &work_id);
    let omitted: Vec<_> = pages
        .iter()
        .flat_map(|page| page["omitted"].as_array().unwrap())
        .collect();
    assert_eq!(omitted.len(), 1, "{omitted:?}");
    assert_eq!(omitted[0]["kind"], "RETAINED_OPERATION");
    assert_eq!(omitted[0]["id"], entrypoint);
    assert_eq!(omitted[0]["reason"], "ITEM_EXCEEDS_WORK_BYTE_BUDGET");
    assert!(omitted[0]["reference"].is_null());

    let first = successful_json(part(&f, &work_id, &entrypoint, "RETAINED_OPERATION", None));
    assert_eq!(first["work"], work_id);
    assert_eq!(first["snapshot"], snapshot);
    assert_eq!(first["recordDigest"], canonical::hash(&original).unwrap());
    assert_eq!(
        first["totalRecordBytes"].as_u64().unwrap() as usize,
        expected.len()
    );
    let first_cursor = first["nextCursor"]
        .as_str()
        .expect("oversized record requires continuation")
        .to_owned();
    let reads_path = f.docs.join(format!(".codeclew/work/{work_id}/reads.json"));
    let interrupted_reads = fs::read(&reads_path).unwrap();
    // Repeat after a separate process invocation: it cannot replace missing parts.
    assert_eq!(
        successful_json(part(&f, &work_id, &entrypoint, "RETAINED_OPERATION", None)),
        first
    );
    assert_eq!(fs::read(&reads_path).unwrap(), interrupted_reads);
    let replacement = "Reserve the normalized requested quantity";
    let proposal = json!({"schema":"codeclew-documentation-proposal/1.0", "operations":[],
        "retainedEdits":[{"kind":"RETAINED_OPERATION", "id":entrypoint,
            "recordDigest":first["recordDigest"], "target":"operationTitle",
            "expectedOldValue":original.title, "replacement":replacement}]});
    let partial = submit(&f, &work_id, &proposal);
    assert_eq!(partial["status"], "NEEDS_REPAIR", "{partial}");
    assert!(
        partial["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["kind"] == "DIAGNOSTIC"
                && item["record"]["code"] == "REQUIRED_CONTEXT_NOT_READ")
    );

    let mut text = first["text"].as_str().unwrap().to_owned();
    let mut cursor = Some(first_cursor.clone());
    while let Some(saved_cursor) = cursor {
        let response = successful_json(part(
            &f,
            &work_id,
            &entrypoint,
            "RETAINED_OPERATION",
            Some(&saved_cursor),
        ));
        assert_eq!(response["startByte"].as_u64().unwrap() as usize, text.len());
        assert_eq!(response["recordDigest"], first["recordDigest"]);
        assert_eq!(response["work"], work_id);
        assert_eq!(response["snapshot"], snapshot);
        let fragment = response["text"].as_str().unwrap();
        assert!(!fragment.is_empty());
        assert_eq!(
            response["fragmentDigest"],
            canonical::hash_bytes(fragment.as_bytes())
        );
        text.push_str(fragment);
        assert_eq!(response["endByte"].as_u64().unwrap() as usize, text.len());
        let next = response["nextCursor"].as_str().map(str::to_owned);
        assert!(next.as_deref() != Some(saved_cursor.as_str()));
        cursor = next;
    }
    assert_eq!(text.as_bytes(), expected);
    assert_eq!(serde_json::from_str::<Operation>(&text).unwrap(), original);
    let ready = submit(&f, &work_id, &proposal);
    assert!(
        ready["status"].as_str().unwrap().starts_with("READY_"),
        "{ready}"
    );
    let artifact = proposals::load(&repo, ready["proposal"].as_str().unwrap()).unwrap();
    assert_eq!(artifact.meaning_review, "UNASSESSED");
    let edited = artifact
        .narrative
        .as_ref()
        .unwrap()
        .operations
        .iter()
        .find(|operation| operation.id == entrypoint)
        .unwrap();
    let mut expected_edit = original.clone();
    expected_edit.title = replacement.into();
    assert_eq!(
        edited, &expected_edit,
        "all unrelated authored fields must survive the bounded edit"
    );

    let other_work = prepare(
        &f,
        &snapshot,
        &entrypoint,
        "Another immutable Work with the same retained operation",
    );
    assert_ne!(other_work, work_id);
    let other_manifest = f
        .docs
        .join(format!(".codeclew/work/{other_work}/work.json"));
    let other_before = fs::read(&other_manifest).unwrap();
    let other_reads = f
        .docs
        .join(format!(".codeclew/work/{other_work}/reads.json"));
    let other_reads_before = fs::read(&other_reads).unwrap();
    let reads_before_rejection = fs::read(&reads_path).unwrap();
    for (rejected, expected_error) in [
        (
            part(&f, &work_id, &entrypoint, "DEPENDENCY", None),
            "retained-part request requires",
        ),
        (
            part(
                &f,
                &work_id,
                "unknown-operation",
                "RETAINED_OPERATION",
                None,
            ),
            "retained operation is outside the Work's selected entrypoint",
        ),
        (
            part(
                &f,
                &other_work,
                &entrypoint,
                "RETAINED_OPERATION",
                Some(&first_cursor),
            ),
            "retained-part cursor belongs to another Work",
        ),
    ] {
        assert!(
            !rejected.status.success(),
            "{}",
            String::from_utf8_lossy(&rejected.stdout)
        );
        let error: Value = serde_json::from_slice(&rejected.stdout).unwrap();
        assert!(error.to_string().contains(expected_error), "{error}");
    }
    assert_eq!(fs::read(&reads_path).unwrap(), reads_before_rejection);
    assert_eq!(fs::read(&other_reads).unwrap(), other_reads_before);
    assert_eq!(fs::read(&work_manifest).unwrap(), work_before);
    assert_eq!(fs::read(&other_manifest).unwrap(), other_before);
    assert_eq!(
        fs::read(f.docs.join("docs/index.html")).unwrap(),
        index_before
    );
    assert_eq!(
        fs::read(f.docs.join(".codeclew/cache/latest-check.json")).unwrap(),
        latest_before
    );
    assert_eq!(bundle_files(&f.bundle(&bundle, "")), frozen_bundle);
    assert_eq!(
        f.ok(&["docs", "history", "show", "--id", &bundle])["status"],
        "FROZEN_SNAPSHOT"
    );
    let frozen = work::load(&repo, &work_id).unwrap();
    assert_eq!(
        frozen
            .retained
            .as_ref()
            .unwrap()
            .operations
            .iter()
            .find(|operation| operation.id == entrypoint)
            .unwrap(),
        &original
    );
    // READY is a deterministic submission result. This test never dispatches an
    // author/reviewer or publishes the edit, so it does not assert reviewed UX-01
    // publication acceptance or bypass it using --unassessed.
}

fn complete_retained(f: &Fixture, work: &str, id: &str) {
    read_pages(f, work);
    let mut cursor = None;
    loop {
        let response = successful_json(part(f, work, id, "RETAINED_OPERATION", cursor.as_deref()));
        cursor = response["nextCursor"].as_str().map(str::to_owned);
        if cursor.is_none() {
            break;
        }
    }
}

#[test]
fn manual_paragraph_edit_preserves_history_authorship_and_original_code_on_source_update() {
    let f = Fixture::new();
    let source_repo = f.service("orders");
    let (_, result) = f.run(&["docs", "check"]);
    let snapshot = result["snapshot"].as_str().unwrap();
    let repo = Repository::open(&f.docs).unwrap();
    let checked = Check::load_snapshot(&repo, snapshot).unwrap();
    let mut narrative = read(f.author("orders", &checked));
    narrative["operations"][0]["documentationLanguage"] = json!("en");
    let template = narrative["operations"][0]["explanation"][0].clone();
    for index in 0..14 {
        let mut paragraph = template.clone();
        paragraph["id"] = json!(format!("retained-detail-{index}"));
        paragraph["text"] = json!(format!(
            "Retained explanation {index}. {}",
            "The fixture keeps its unrelated explanation intact. ".repeat(100)
        ));
        paragraph["detail"] = json!(true);
        narrative["operations"][0]["explanation"]
            .as_array_mut()
            .unwrap()
            .push(paragraph);
    }
    let original: Operation = serde_json::from_value(narrative["operations"][0].clone()).unwrap();
    assert!(canonical::bytes(&original).unwrap().len() > MAX_BYTES);
    let id = original.id.as_str();
    let input = f.input("paragraph-baseline.json", &narrative);
    let baseline = f.ok(&[
        "docs",
        "render",
        "--snapshot",
        snapshot,
        "--input",
        input.to_str().unwrap(),
        "--publish",
    ]);
    assert_eq!(baseline["updateFailures"], json!({}));
    let old_bundle = baseline["bundle"].as_str().unwrap();
    let old_bytes = bundle_files(&f.bundle(old_bundle, ""));
    let old_data = read(f.bundle(old_bundle, "services/orders.json"));
    let work = prepare(&f, snapshot, id, "Manual paragraph correction");
    let concurrent = prepare(&f, snapshot, id, "Concurrent manual paragraph correction");
    complete_retained(&f, &work, id);
    complete_retained(&f, &concurrent, id);
    let replacement = "The caller supplies a quantity. This explanation is maintained by the documentation editor, and the linked code is context rather than proof.";
    let proposal = json!({"schema":"codeclew-documentation-proposal/1.0", "operations":[],
        "retainedEdits":[{"kind":"RETAINED_OPERATION", "id":id, "recordDigest":canonical::hash(&original).unwrap(),
            "target":"explanationText", "fragmentId":original.explanation[0].id, "author":"Fixture editor",
            "expectedOldValue":original.explanation[0].text, "replacement":replacement}]});
    let submitted = submit(&f, &work, &proposal);
    assert!(
        submitted["status"].as_str().unwrap().starts_with("READY_"),
        "{submitted}"
    );
    let concurrent_proposal = submit(&f, &concurrent, &proposal);
    assert!(
        concurrent_proposal["status"]
            .as_str()
            .unwrap()
            .starts_with("READY_"),
        "{concurrent_proposal}"
    );
    let publication = f.ok(&[
        "docs",
        "proposal",
        "publish",
        "--proposal",
        submitted["proposal"].as_str().unwrap(),
        "--unassessed",
    ]);
    assert_eq!(publication["meaningReview"], "UNASSESSED");
    assert_eq!(publication["updateFailures"], json!({}), "{publication}");
    let new_bundle = publication["bundle"].as_str().unwrap();
    let data = read(f.bundle(new_bundle, "services/orders.json"));
    let actual: Operation = serde_json::from_value(data["operations"][0].clone()).unwrap();
    let mut expected = original.clone();
    expected.explanation[0].text = replacement.into();
    expected.explanation[0].authorship = actual.explanation[0].authorship.clone();
    assert_eq!(
        actual, expected,
        "only text and declared authorship may change"
    );
    let authorship = actual.explanation[0].authorship.as_ref().unwrap();
    assert_eq!(authorship.author, "Fixture editor");
    let serialized = serde_json::to_value(authorship).unwrap();
    assert_eq!(serialized["authority"], "USER_DOCUMENTATION");
    assert_eq!(serialized["meaningReview"], "UNASSESSED");
    assert_eq!(serialized["contextRole"], "RETAINED_UNVERIFIED_CONTEXT");
    assert_eq!(authorship.source_snapshot, snapshot);
    assert_eq!(data["sources"], old_data["sources"]);
    assert_eq!(data["operationSources"], old_data["operationSources"]);
    assert_eq!(data["operationStates"][id]["verification"], "UNASSESSED");
    let markdown = fs::read_to_string(f.bundle(new_bundle, "services/orders.md")).unwrap();
    assert!(markdown.contains("User documentation by Fixture editor. Meaning review: UNASSESSED."));
    let html = fs::read_to_string(f.bundle(new_bundle, "services/orders.html")).unwrap();
    assert!(html.contains("Originally linked code (unverified context)"));
    assert!(html.contains("Linked code does not verify the narrative meaning."));
    assert_eq!(bundle_files(&f.bundle(old_bundle, "")), old_bytes);
    assert_eq!(
        f.ok(&["docs", "history", "show", "--id", old_bundle])["status"],
        "FROZEN_SNAPSHOT"
    );
    let current_index = fs::read(f.docs.join("docs/index.html")).unwrap();
    let (code, conflict) = f.run(&[
        "docs",
        "proposal",
        "publish",
        "--proposal",
        concurrent_proposal["proposal"].as_str().unwrap(),
        "--unassessed",
    ]);
    assert_ne!(code, 0);
    assert!(
        conflict
            .to_string()
            .contains("published content changed after work preparation"),
        "{conflict}"
    );
    assert_eq!(
        fs::read(f.docs.join("docs/index.html")).unwrap(),
        current_index
    );
    // Direct narrative input uses the current publication context, including its
    // registered-input scope, so rejection exercises authored-text protection.
    narrative["contextDigest"] = publication["contextDigest"].clone();
    let input = f.input("paragraph-overwrite.json", &narrative);
    let overwritten = f.ok(&[
        "docs",
        "render",
        "--snapshot",
        snapshot,
        "--input",
        input.to_str().unwrap(),
        "--publish",
    ]);
    assert!(
        overwritten["updateFailures"]
            .to_string()
            .contains("protected user-authored explanation"),
        "{overwritten}"
    );
    let preserved = read(f.bundle(
        overwritten["bundle"].as_str().unwrap(),
        "services/orders.json",
    ));
    assert_eq!(preserved["operations"], data["operations"]);
    let mut forged = narrative.clone();
    forged["operations"][0] = serde_json::to_value(&actual).unwrap();
    forged["operations"][0]["explanation"][0]["text"] =
        json!("An unsupported direct narrative edit.");
    forged["operations"][0]["explanation"][0]["authorship"]["author"] =
        json!("Forged fixture identity");
    let forged_input = f.input("forged-authorship.json", &forged);
    let rejected = f.ok(&[
        "docs",
        "render",
        "--snapshot",
        snapshot,
        "--input",
        forged_input.to_str().unwrap(),
        "--publish",
    ]);
    assert!(
        rejected["updateFailures"]
            .to_string()
            .contains("bound manual UNASSESSED proposal"),
        "{rejected}"
    );
    let forged_rejection = rejected.clone();
    // The supported status update observes changed source without changing authored
    // text, its declared identity, or the original linked bytes.
    let source = source_repo.join("Orders.java");
    fs::write(
        &source,
        fs::read_to_string(&source)
            .unwrap()
            .replace("return quantity;", "return quantity + 1;"),
    )
    .unwrap();
    support::commit(&source_repo);
    let refreshed = f.ok(&["docs", "refresh", "--status-only"]);
    assert_eq!(refreshed["agentInvocations"], 0);
    let stale = read(f.bundle(
        refreshed["bundle"].as_str().unwrap(),
        "services/orders.json",
    ));
    assert_eq!(stale["operations"], data["operations"]);
    assert_eq!(stale["sources"], data["sources"]);
    assert_eq!(stale["operationSources"], data["operationSources"]);
    assert_eq!(stale["operationStates"][id]["freshness"], "STALE");
    assert_eq!(stale["operationStates"][id]["verification"], "UNASSESSED");
    let (_, recaptured) = f.run(&["docs", "check"]);
    let changed_snapshot = recaptured["snapshot"].as_str().unwrap();
    assert_ne!(changed_snapshot, snapshot);
    let changed_work = prepare(
        &f,
        changed_snapshot,
        id,
        "Refuse silent paragraph source rebinding",
    );
    complete_retained(&f, &changed_work, id);
    let mut changed_proposal = proposal.clone();
    changed_proposal["retainedEdits"][0]["recordDigest"] = json!(canonical::hash(&actual).unwrap());
    changed_proposal["retainedEdits"][0]["expectedOldValue"] = json!(replacement);
    changed_proposal["retainedEdits"][0]["replacement"] =
        json!("A new editor correction against changed source.");
    let preserved_context_edit = submit(&f, &changed_work, &changed_proposal);
    assert!(
        preserved_context_edit["status"]
            .as_str()
            .unwrap()
            .starts_with("READY_"),
        "{preserved_context_edit}"
    );
    let next_artifact =
        proposals::load(&repo, preserved_context_edit["proposal"].as_str().unwrap()).unwrap();
    let next_paragraph = &next_artifact.narrative.as_ref().unwrap().operations[0].explanation[0];
    let next_authorship = next_paragraph.authorship.as_ref().unwrap();
    assert_eq!(next_authorship.source_snapshot, authorship.source_snapshot);
    assert_eq!(next_authorship.source_refs, authorship.source_refs);
    assert_eq!(next_authorship.dependency_refs, authorship.dependency_refs);
    let new_check = Check::load_snapshot(&repo, changed_snapshot).unwrap();
    let regenerated = f.author("orders", &new_check);
    let current_context = f.ok(&[
        "docs",
        "render",
        "--snapshot",
        changed_snapshot,
        "--publish",
    ]);
    let mut regenerated_value = read(&regenerated);
    regenerated_value["contextDigest"] = current_context["contextDigest"].clone();
    let regenerated = f.input("paragraph-regeneration.json", &regenerated_value);
    let rejected = f.ok(&[
        "docs",
        "render",
        "--snapshot",
        changed_snapshot,
        "--input",
        regenerated.to_str().unwrap(),
        "--publish",
    ]);
    assert!(
        rejected["updateFailures"]
            .to_string()
            .contains("protected user-authored explanation"),
        "{rejected}"
    );
    let final_data = read(f.bundle(rejected["bundle"].as_str().unwrap(), "services/orders.json"));
    assert_eq!(final_data["operations"], data["operations"]);
    assert_eq!(final_data["operationSources"], data["operationSources"]);
    assert_eq!(final_data["operationStates"][id]["freshness"], "STALE");
    assert_eq!(bundle_files(&f.bundle(old_bundle, "")), old_bytes);
    if let Some(destination) = std::env::var_os("CODECLEW_PARAGRAPH_TEST_ARTIFACTS") {
        fn copy_tree(source: &Path, target: &Path) {
            fs::create_dir_all(target).unwrap();
            for entry in fs::read_dir(source).unwrap() {
                let entry = entry.unwrap();
                let output = target.join(entry.file_name());
                if entry.file_type().unwrap().is_dir() {
                    copy_tree(&entry.path(), &output);
                } else {
                    assert!(entry.file_type().unwrap().is_file());
                    fs::copy(entry.path(), output).unwrap();
                }
            }
        }
        let destination = Path::new(&destination);
        assert!(
            !destination.exists(),
            "artifact sink must be a new directory"
        );
        copy_tree(&f.docs, &destination.join("docs-root"));
        for entry in fs::read_dir(f.temp.path()).unwrap() {
            let entry = entry.unwrap();
            if entry
                .path()
                .extension()
                .is_some_and(|extension| extension == "json")
            {
                fs::copy(entry.path(), destination.join(entry.file_name())).unwrap();
            }
        }
        fs::write(destination.join("journey.json"), serde_json::to_vec_pretty(&json!({
            "schema":"codeclew-synthetic-paragraph-journey/1.0",
            "authority":"PUBLIC_MANUAL_CLI_UNASSESSED_NOT_MEANING_REVIEW_QUALIFICATION",
            "baselineSnapshot":snapshot, "changedSnapshot":changed_snapshot,
            "operation":id, "fragment":original.explanation[0].id,
            "baselinePublication":baseline, "paragraphSubmission":submitted,
            "paragraphPublication":publication, "concurrentPublicationConflict":conflict,
            "directOverwriteRejection":overwritten, "forgedAuthorshipRejection":forged_rejection,
            "sourceStatusUpdate":refreshed, "changedContextSubmission":preserved_context_edit,
            "finalRegenerationRejection":rejected,
            "commands":["docs render --snapshot BASELINE --input paragraph-baseline.json --publish",
                "docs work prepare --subject service:orders --snapshot BASELINE --input retained-work-request.json",
                "docs work read --work WORK --input retained-work-selection.json (all pages)",
                "docs work read-retained-part --work WORK --input retained-part-request.json (all parts)",
                "docs proposal submit --work WORK --input retained-title-proposal.json",
                "docs proposal publish --proposal PROPOSAL --unassessed",
                "docs refresh --status-only", "docs check",
                "docs render --snapshot CHANGED --input paragraph-regeneration.json --publish"]
        })).unwrap()).unwrap();
    }
}

fn source_operation_proposal(work: &work::Work, summary: &str, step: &str, large: bool) -> Value {
    let id = work.request.entrypoint.as_deref().unwrap();
    let reference = work
        .handles
        .iter()
        .find(|(_, handle)| handle.kind == "ENTRYPOINT" && handle.id == id)
        .unwrap()
        .0;
    let entry = work.checked.services["orders"]
        .entrypoints
        .iter()
        .find(|e| e.id == id)
        .unwrap();
    let return_flow = work
        .handles
        .iter()
        .find(|(_, handle)| {
            handle.kind == "DEPENDENCY"
                && work.checked.dependencies.get(&handle.id).is_some_and(|d| {
                    d.kind == "FLOW" && d.symbol == entry.symbol && d.normalized["kind"] == "RETURN"
                })
        })
        .unwrap()
        .0;
    let claim = |text: String| json!({"text":text,"evidence":[reference]});
    let step_claim = json!({"text":step,"evidence":[reference,return_flow]});
    let explanation: Vec<_> = if large {
        (0..14)
            .map(|i| {
                claim(format!(
                    "Maintained fixture explanation {i}. {}",
                    "The editor keeps this synthetic documentation paragraph intact. ".repeat(85)
                ))
            })
            .collect()
    } else {
        vec![claim(
            "Current source-derived explanation: return normalized quantity plus one.".into(),
        )]
    };
    json!({"schema":"codeclew-documentation-proposal/1.0","operations":[{
        "entrypoint":reference,"title":"Quantity handling", "summary":claim(summary.into()),
        "steps":[{"kind":"note","meaning":step_claim}], "explanation":explanation
    }]})
}

#[test]
fn mixed_source_regeneration_preserves_oversized_user_text_and_exact_original_fragment_code() {
    let f = Fixture::new();
    let source_repo = f.service("orders");
    let (_, first_check) = f.run(&["docs", "check"]);
    let old_snapshot = first_check["snapshot"].as_str().unwrap();
    let repo = Repository::open(&f.docs).unwrap();
    let checked = Check::load_snapshot(&repo, old_snapshot).unwrap();
    let entry = checked.services["orders"]
        .entrypoints
        .iter()
        .find(|e| e.symbol.contains("reserve"))
        .unwrap();
    let id = entry.id.as_str();
    let initial_work = prepare(&f, old_snapshot, id, "Seed a large manual operation");
    read_pages(&f, &initial_work);
    let initial_input = source_operation_proposal(
        &work::load(&repo, &initial_work).unwrap(),
        "The source returns a normalized quantity.",
        "Return the normalized requested quantity.",
        true,
    );
    let seed = submit(&f, &initial_work, &initial_input);
    assert!(
        seed["status"].as_str().unwrap().starts_with("READY_"),
        "{seed}"
    );
    let seed_publication = f.ok(&[
        "docs",
        "proposal",
        "publish",
        "--proposal",
        seed["proposal"].as_str().unwrap(),
        "--unassessed",
    ]);
    assert_eq!(
        seed_publication["updateFailures"],
        json!({}),
        "{seed_publication}"
    );
    let seeded_data = read(f.bundle(
        seed_publication["bundle"].as_str().unwrap(),
        "services/orders.json",
    ));
    let original: Operation = serde_json::from_value(seeded_data["operations"][0].clone()).unwrap();
    assert!(canonical::bytes(&original).unwrap().len() > MAX_BYTES);
    let human_work = prepare(&f, old_snapshot, id, "Protect maintained explanation text");
    complete_retained(&f, &human_work, id);
    let protected_input = json!({"schema":"codeclew-documentation-proposal/1.0","operations":[],
        "retainedEdits":original.explanation.iter().map(|p| json!({"kind":"RETAINED_OPERATION", "id":id,
            "recordDigest":canonical::hash(&original).unwrap(), "target":"explanationText", "fragmentId":p.id,
            "author":"Fixture documentation editor", "expectedOldValue":p.text, "replacement":p.text})).collect::<Vec<_>>()});
    let human_submission = submit(&f, &human_work, &protected_input);
    assert!(
        human_submission["status"]
            .as_str()
            .unwrap()
            .starts_with("READY_"),
        "{human_submission}"
    );
    let human_publication = f.ok(&[
        "docs",
        "proposal",
        "publish",
        "--proposal",
        human_submission["proposal"].as_str().unwrap(),
        "--unassessed",
    ]);
    assert_eq!(
        human_publication["updateFailures"],
        json!({}),
        "{human_publication}"
    );
    let old_bundle = human_publication["bundle"].as_str().unwrap();
    let frozen = bundle_files(&f.bundle(old_bundle, ""));
    let old_binding = clew::documentation::bindings::baseline(&repo)
        .unwrap()
        .unwrap()
        .1;
    let protected_operation = &old_binding.narratives["service:orders"].operations[0];
    assert!(
        protected_operation
            .explanation
            .iter()
            .all(|p| p.authorship.is_some())
    );
    assert!(canonical::bytes(protected_operation).unwrap().len() > MAX_BYTES);
    let old_source_id = protected_operation.summary.source_ids[0].clone();
    let old_sources = checked.sources();
    let old_source = &old_sources[&old_source_id];
    let source = source_repo.join("Orders.java");
    fs::write(
        &source,
        fs::read_to_string(&source).unwrap().replace(
            "return normalize(quantity);",
            "return normalize(quantity) + 1;",
        ),
    )
    .unwrap();
    support::commit(&source_repo);
    let observed = f.ok(&["docs", "refresh", "--status-only"]);
    assert_eq!(observed["agentInvocations"], 0);
    let (_, changed_check) = f.run(&["docs", "check"]);
    let new_snapshot = changed_check["snapshot"].as_str().unwrap();
    let current_check = Check::load_snapshot(&repo, new_snapshot).unwrap();
    let current_sources = current_check.sources();
    let current_source = &current_sources[&old_source_id];
    assert_eq!(
        old_source.id, current_source.id,
        "logical SOURCE identity stays unchanged"
    );
    assert_ne!(old_source.text, current_source.text);
    assert!(current_source.text.contains("+ 1"));
    let regeneration_work = prepare(
        &f,
        new_snapshot,
        id,
        "Regenerate current source fields and preserve protected text",
    );
    let concurrent_work = prepare(&f, new_snapshot, id, "Concurrent regenerated source fields");
    complete_retained(&f, &regeneration_work, id);
    complete_retained(&f, &concurrent_work, id);
    let current_summary = "The source adds one to the normalized requested quantity.";
    let current_step = "Return normalized quantity plus one.";
    let regenerated_input = source_operation_proposal(
        &work::load(&repo, &regeneration_work).unwrap(),
        current_summary,
        current_step,
        false,
    );
    assert!(
        !regenerated_input.to_string().contains("authorship"),
        "model input never fabricates protected authorship"
    );
    let regenerated = submit(&f, &regeneration_work, &regenerated_input);
    let concurrent = submit(&f, &concurrent_work, &regenerated_input);
    assert!(
        regenerated["status"]
            .as_str()
            .unwrap()
            .starts_with("READY_"),
        "{regenerated}"
    );
    assert!(
        concurrent["status"].as_str().unwrap().starts_with("READY_"),
        "{concurrent}"
    );
    let artifact = proposals::load(&repo, regenerated["proposal"].as_str().unwrap()).unwrap();
    let canonical = &artifact.narrative.as_ref().unwrap().operations[0];
    assert_eq!(canonical.summary.text, current_summary);
    assert_eq!(canonical.events[0].text, current_step);
    assert!(canonical::bytes(canonical).unwrap().len() > MAX_BYTES);
    for paragraph in &protected_operation.explanation {
        assert_eq!(
            canonical
                .explanation
                .iter()
                .find(|p| p.id == paragraph.id)
                .unwrap(),
            paragraph
        );
    }
    assert!(
        artifact
            .claims
            .values()
            .all(|c| c["authority"] != "USER_DOCUMENTATION"),
        "preserved prose is not relisted as new source-supported claims"
    );
    let published = f.ok(&[
        "docs",
        "proposal",
        "publish",
        "--proposal",
        regenerated["proposal"].as_str().unwrap(),
        "--unassessed",
    ]);
    assert_eq!(published["updateFailures"], json!({}), "{published}");
    let new_bundle = published["bundle"].as_str().unwrap();
    let data_path = f.bundle(new_bundle, "services/orders.json");
    let data = read(&data_path);
    let new_binding = clew::documentation::bindings::baseline(&repo)
        .unwrap()
        .unwrap()
        .1;
    let operation_key = format!("service:orders/{id}");
    let current_scope = &new_binding.accepted_versions[&operation_key]
        .influence
        .scope;
    let generated_summary_key = format!("{operation_key}/{}", canonical.summary.id);
    assert_eq!(
        new_binding.fragments[&generated_summary_key]
            .influence_scope
            .as_ref(),
        Some(current_scope)
    );
    assert_eq!(
        data["operationSources"][id][&old_source_id]["text"],
        current_source.text
    );
    for paragraph in &protected_operation.explanation {
        let key = format!("{operation_key}/{}", paragraph.id);
        assert_eq!(
            canonical::bytes(&new_binding.fragments[&key]).unwrap(),
            canonical::bytes(&old_binding.fragments[&key]).unwrap(),
            "the complete paragraph binding remains original"
        );
        assert_ne!(
            new_binding.fragments[&key].influence_scope.as_ref(),
            Some(current_scope)
        );
        assert_eq!(
            data["fragmentSources"][&key][&old_source_id]["text"],
            old_source.text
        );
        assert_eq!(data["fragmentStates"][&key]["freshness"], "STALE");
        assert_eq!(data["fragmentStates"][&key]["verification"], "UNASSESSED");
    }
    let mixed = data["operationStates"][id]["mixedRevisions"]["orders"]
        .as_array()
        .unwrap();
    assert!(
        mixed.contains(&json!(old_source.revision))
            && mixed.contains(&json!(current_source.revision)),
        "{mixed:?}"
    );
    let markdown = fs::read_to_string(f.bundle(new_bundle, "services/orders.md")).unwrap();
    assert!(markdown.contains("source context freshness STALE. Meaning review: UNASSESSED."));
    assert!(
        fs::read_to_string(f.bundle(new_bundle, "services/orders.html"))
            .unwrap()
            .contains("data-source-fragment")
    );
    // Execute the shipped drawer against the actual published JSON, including
    // both versions of the same SOURCE id and the exact missing-fragment route.
    let reader_script = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../scripts/test_documentation_visual_reader.cjs");
    let drawer = std::process::Command::new("node")
        .args(["--test", "--test-name-pattern", "public mixed publication"])
        .arg(&reader_script)
        .env("CODECLEW_MIXED_READER_DATA", &data_path)
        .output()
        .unwrap();
    assert!(
        drawer.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&drawer.stdout),
        String::from_utf8_lossy(&drawer.stderr)
    );
    let index_before = fs::read(f.docs.join("docs/index.html")).unwrap();
    let (code, conflict) = f.run(&[
        "docs",
        "proposal",
        "publish",
        "--proposal",
        concurrent["proposal"].as_str().unwrap(),
        "--unassessed",
    ]);
    assert_ne!(code, 0);
    assert!(
        conflict
            .to_string()
            .contains("published content changed after work preparation"),
        "{conflict}"
    );
    assert_eq!(
        fs::read(f.docs.join("docs/index.html")).unwrap(),
        index_before
    );
    let repeated = f.ok(&["docs", "refresh", "--status-only"]);
    let repeated_data =
        read(f.bundle(repeated["bundle"].as_str().unwrap(), "services/orders.json"));
    assert_eq!(repeated_data["operations"], data["operations"]);
    assert_eq!(repeated_data["fragmentSources"], data["fragmentSources"]);
    assert_eq!(repeated_data["fragmentStates"], data["fragmentStates"]);
    assert_eq!(bundle_files(&f.bundle(old_bundle, "")), frozen);
    assert_eq!(
        f.ok(&["docs", "history", "show", "--id", old_bundle])["status"],
        "FROZEN_SNAPSHOT"
    );
    // Later user text edits keep old context even though Work uses current code.
    let edit_work = prepare(
        &f,
        new_snapshot,
        id,
        "Correct preserved text without rebasing context",
    );
    complete_retained(&f, &edit_work, id);
    let mixed_operation: Operation =
        serde_json::from_value(repeated_data["operations"][0].clone()).unwrap();
    let paragraph = &protected_operation.explanation[0];
    let text_edit = json!({"schema":"codeclew-documentation-proposal/1.0","operations":[],"retainedEdits":[{
        "kind":"RETAINED_OPERATION","id":id,"recordDigest":canonical::hash(&mixed_operation).unwrap(),
        "target":"explanationText","fragmentId":paragraph.id,"author":"Fixture documentation editor",
        "expectedOldValue":paragraph.text,"replacement":"A later explicit text correction retains the originally linked code."}]});
    let edited = submit(&f, &edit_work, &text_edit);
    assert!(
        edited["status"].as_str().unwrap().starts_with("READY_"),
        "{edited}"
    );
    let edited_artifact = proposals::load(&repo, edited["proposal"].as_str().unwrap()).unwrap();
    let edited_paragraph = edited_artifact.narrative.as_ref().unwrap().operations[0]
        .explanation
        .iter()
        .find(|p| p.id == paragraph.id)
        .unwrap();
    assert_eq!(
        edited_paragraph
            .authorship
            .as_ref()
            .unwrap()
            .source_snapshot,
        old_snapshot
    );
    assert_eq!(
        edited_paragraph.authorship.as_ref().unwrap().source_refs,
        paragraph.authorship.as_ref().unwrap().source_refs
    );
    let edited_publication = f.ok(&[
        "docs",
        "proposal",
        "publish",
        "--proposal",
        edited["proposal"].as_str().unwrap(),
        "--unassessed",
    ]);
    assert_eq!(
        edited_publication["updateFailures"],
        json!({}),
        "{edited_publication}"
    );
    let edited_data = read(f.bundle(
        edited_publication["bundle"].as_str().unwrap(),
        "services/orders.json",
    ));
    let paragraph_key = format!("{operation_key}/{}", paragraph.id);
    assert_eq!(
        edited_data["fragmentSources"][&paragraph_key][&old_source_id]["text"],
        old_source.text
    );
    assert_eq!(
        edited_data["operationSources"][id][&old_source_id]["text"],
        current_source.text
    );
    assert_eq!(
        edited_data["fragmentStates"][&paragraph_key]["freshness"],
        "STALE"
    );
    assert_eq!(
        edited_data["fragmentStates"][&paragraph_key]["verification"],
        "UNASSESSED"
    );
    let edited_binding = clew::documentation::bindings::baseline(&repo)
        .unwrap()
        .unwrap()
        .1;
    assert_eq!(
        canonical::bytes(&edited_binding.fragments[&paragraph_key].evidence).unwrap(),
        canonical::bytes(&old_binding.fragments[&paragraph_key].evidence).unwrap()
    );
    assert_eq!(
        edited_binding.fragments[&paragraph_key].influence_scope,
        old_binding.fragments[&paragraph_key].influence_scope
    );
    assert_eq!(bundle_files(&f.bundle(old_bundle, "")), frozen);
    if let Some(destination) = std::env::var_os("CODECLEW_MIXED_TEST_ARTIFACTS") {
        let destination = Path::new(&destination);
        assert!(
            !destination.exists(),
            "mixed artifact sink must be a new directory"
        );
        fs::create_dir_all(destination).unwrap();
        for (path, content) in bundle_files(&f.docs) {
            let target = destination.join("docs-root").join(path);
            fs::create_dir_all(target.parent().unwrap()).unwrap();
            fs::write(target, content).unwrap();
        }
        fs::write(destination.join("reader.log"), &drawer.stdout).unwrap();
        fs::write(destination.join("journey.json"), serde_json::to_vec_pretty(&json!({
            "schema":"codeclew-synthetic-mixed-source-journey/1.0", "authority":"PUBLIC_MANUAL_CLI_UNASSESSED",
            "oldSnapshot":old_snapshot,"newSnapshot":new_snapshot,"operation":id,"source":old_source_id,
            "seedPublication":seed_publication,"protectedPublication":human_publication,"sourceStatusObservation":observed,
            "generatedInput":regenerated_input,"generatedSubmission":regenerated,"mixedPublication":published,
            "concurrentConflict":conflict,"repeatedStatusUpdate":repeated,"laterTextEditInput":text_edit,"laterTextEditSubmission":edited,"laterTextEditPublication":edited_publication,
            "commands":["docs check", "docs work prepare --subject service:orders --snapshot SNAPSHOT --input request.json",
                "docs work read --work WORK --input selection.json (all pages)", "docs work read-retained-part --work WORK --input part.json (all parts)",
                "docs proposal submit --work WORK --input proposal.json", "docs proposal publish --proposal PROPOSAL --unassessed",
                "docs refresh --status-only", "docs history show --id ORIGINAL_BUNDLE"]
        })).unwrap()).unwrap();
    }
}
