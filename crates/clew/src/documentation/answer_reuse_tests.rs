//! Discovery through real immutable stores and deterministic local role drivers.
#![cfg(target_os = "macos")]

use super::{
    agent_jobs::{self, operation_draft_review},
    answer_reuse,
    store::{self, Repository},
    work::{self, Work},
};
use serde_json::{Value, json};
use std::{collections::BTreeMap, fs, path::PathBuf};

struct ApprovedFixture {
    _temp: tempfile::TempDir,
    repo: Repository,
    work: Work,
    author_config: PathBuf,
    author_run: String,
    review_run: String,
}

fn approved_fixture() -> ApprovedFixture {
    let (temp, repo, mut seed, author_config) = agent_jobs::reusable_author_fixture_setup();
    seed.request.entrypoint = None;
    seed.request.context_profile = Some("process-graph-v1".into());
    seed.request.root_declaration = Some("endpoint-declaration".into());
    seed.request.question = Some("Where does this internal operation obtain its result?".into());
    seed.request.authoring_contract = Some("codeclew-operation-draft-authoring/1.6".into());
    seed.request.source_data_context = true;
    work::api_contract_tests::persist_operation_fixture(&repo, &mut seed);
    let snapshot = seed.snapshot.as_deref().unwrap();
    // Prepare normally so initial selection, obligations and input membership
    // match the preparation replay used by discovery.
    let prepared = work::prepare_with_snapshot(
        &repo,
        seed.subject.clone(),
        seed.request.clone(),
        Some(snapshot),
    )
    .unwrap();
    let work = work::load(&repo, prepared["work"].as_str().unwrap()).unwrap();
    let authored =
        agent_jobs::run_operation_draft(&repo, &work.id, Some(&author_config), false, None, None)
            .unwrap();
    assert_eq!(authored["status"], "DRAFT", "{authored}");
    assert_eq!(authored["attempts"].as_array().unwrap().len(), 1);
    let author_run = authored["run"].as_str().unwrap().to_owned();
    let review_config = temp.path().join("review-config.json");
    agent_jobs::write_reusable_review_fixture_config(&review_config, &author_config, "approve");
    let reviewed =
        agent_jobs::review_operation_draft(&repo, &work.id, &author_run, &review_config, None)
            .unwrap();
    assert_eq!(reviewed["status"], "DRAFT_REVIEW_APPROVED", "{reviewed}");
    assert_eq!(reviewed["attempts"].as_array().unwrap().len(), 1);
    ApprovedFixture {
        _temp: temp,
        repo,
        work,
        author_config,
        author_run,
        review_run: reviewed["run"].as_str().unwrap().to_owned(),
    }
}

fn discover(fixture: &ApprovedFixture) -> Value {
    answer_reuse::find(
        &fixture.repo,
        &fixture.work.subject,
        fixture.work.request.clone(),
        fixture.work.snapshot.as_deref().unwrap(),
        None,
    )
    .unwrap()
}

fn files(path: &std::path::Path, saved: &mut BTreeMap<PathBuf, Vec<u8>>) {
    if path.is_dir() {
        for entry in fs::read_dir(path).unwrap() {
            files(&entry.unwrap().path(), saved);
        }
    } else if path.is_file() {
        saved.insert(path.to_owned(), fs::read(path).unwrap());
    }
}

#[test]
fn exact_approved_answer_discovery_preserves_answer_provenance_and_all_files() {
    let fixture = approved_fixture();
    let approved = operation_draft_review::load_approved_answer(
        &fixture.repo,
        &fixture.work,
        &fixture.review_run,
    )
    .unwrap();
    let mut before = BTreeMap::new();
    files(&fixture.repo.root, &mut before);
    let found = discover(&fixture);
    assert_eq!(found["status"], "FOUND", "{found}");
    assert_eq!(found["searchComplete"], true);
    assert_eq!(found["candidates"].as_array().unwrap().len(), 1);
    assert_eq!(found["candidates"][0]["applicability"], "CURRENT");
    assert_eq!(found["selected"]["answer"], approved.answer);
    assert_eq!(found["selected"]["review"], approved.review);
    assert_eq!(found["selected"]["packet"], approved.packet);
    assert_eq!(found["selected"]["audit"], approved.audit);
    assert_eq!(found["selected"]["provenance"], approved.provenance);
    assert_eq!(
        found["selected"]["selection"]["reviewRun"],
        fixture.review_run
    );
    for field in ["captures", "agentInvocations", "writes"] {
        assert_eq!(found[field], 0);
    }
    let mut after = BTreeMap::new();
    files(&fixture.repo.root, &mut after);
    assert_eq!(before, after);
}

fn second_approval(fixture: &ApprovedFixture) -> Value {
    let original_report_path = fixture
        .repo
        .path(&format!(".codeclew/jobs/{}.json", fixture.review_run))
        .unwrap();
    let original_report = fs::read(&original_report_path).unwrap();
    // Controlled fixture setup selects the genuine author report again, enabling
    // a second real deterministic review without forging any approval report.
    fixture
        .repo
        .atomic(
            &format!(".codeclew/work/{}/latest-run.json", fixture.work.id),
            &serde_json::to_vec(&json!({"run":fixture.author_run})).unwrap(),
        )
        .unwrap();
    let review_config = fixture._temp.path().join("second-review-config.json");
    agent_jobs::write_reusable_review_fixture_config(
        &review_config,
        &fixture.author_config,
        "second-approve",
    );
    let second = agent_jobs::review_operation_draft(
        &fixture.repo,
        &fixture.work.id,
        &fixture.author_run,
        &review_config,
        None,
    )
    .unwrap();
    assert_eq!(second["status"], "DRAFT_REVIEW_APPROVED", "{second}");
    assert_ne!(second["run"], fixture.review_run);
    assert_eq!(fs::read(&original_report_path).unwrap(), original_report);
    second
}

#[test]
fn historical_approvals_beyond_latest_require_explicit_selection() {
    let fixture = approved_fixture();
    let second = second_approval(&fixture);
    let found = discover(&fixture);
    assert_eq!(found["status"], "SELECTION_REQUIRED", "{found}");
    assert!(found["selected"].is_null());
    let candidates = found["candidates"].as_array().unwrap();
    assert_eq!(candidates.len(), 2);
    assert!(
        candidates
            .iter()
            .all(|candidate| candidate["applicability"] == "CURRENT")
    );
    for run in [json!(fixture.review_run), second["run"].clone()] {
        assert!(
            candidates
                .iter()
                .any(|candidate| candidate["selection"]["reviewRun"] == run)
        );
    }
    let selector = serde_json::from_value(candidates[0]["selection"].clone()).unwrap();
    let selected = answer_reuse::find(
        &fixture.repo,
        &fixture.work.subject,
        fixture.work.request.clone(),
        fixture.work.snapshot.as_deref().unwrap(),
        Some(selector),
    )
    .unwrap();
    assert_eq!(selected["status"], "FOUND");
    assert_eq!(
        selected["selected"]["selection"],
        candidates[0]["selection"]
    );
}

#[test]
fn different_exact_question_does_not_reuse_an_approved_answer() {
    let fixture = approved_fixture();
    let mut request = fixture.work.request.clone();
    request.question = Some("Which failure paths occur in this internal operation?".into());
    let found = answer_reuse::find(
        &fixture.repo,
        &fixture.work.subject,
        request,
        fixture.work.snapshot.as_deref().unwrap(),
        None,
    )
    .unwrap();
    assert_eq!(found["status"], "NO_MATCH", "{found}");
    assert!(found["candidates"].as_array().unwrap().is_empty());
    assert!(found["selected"].is_null());
}

#[test]
fn corrupt_historical_report_binding_fails_closed() {
    let fixture = approved_fixture();
    let path = fixture
        .repo
        .path(&format!(".codeclew/jobs/{}.json", fixture.review_run))
        .unwrap();
    let mut report: Value = store::read(&path, store::MAX_RECORD).unwrap();
    report["run"] = json!("f".repeat(32));
    let corrupted = serde_json::to_vec(&report).unwrap();
    fs::write(&path, &corrupted).unwrap();
    let error = answer_reuse::find(
        &fixture.repo,
        &fixture.work.subject,
        fixture.work.request.clone(),
        fixture.work.snapshot.as_deref().unwrap(),
        None,
    )
    .unwrap_err();
    assert!(
        error.message.contains("RECOVERY_REPORT_MISMATCH"),
        "{}",
        error.message
    );
    assert_eq!(fs::read(&path).unwrap(), corrupted);
}

#[test]
fn missing_nonlatest_historical_approval_cannot_become_a_unique_match() {
    let fixture = approved_fixture();
    let second = second_approval(&fixture);
    let pointer = fixture
        .repo
        .path(&format!(
            ".codeclew/work/{}/latest-run.json",
            fixture.work.id
        ))
        .unwrap();
    let latest: Value = store::read(&pointer, store::MAX_RECORD).unwrap();
    assert_eq!(latest["run"], second["run"]);
    let missing = fixture
        .repo
        .path(&format!(".codeclew/jobs/{}.json", fixture.review_run))
        .unwrap();
    let checkpoint_directory = fixture
        .repo
        .path(&format!(".codeclew/jobs/{}", fixture.review_run))
        .unwrap();
    assert!(checkpoint_directory.is_dir());
    fs::remove_file(&missing).unwrap();
    let error = answer_reuse::find(
        &fixture.repo,
        &fixture.work.subject,
        fixture.work.request.clone(),
        fixture.work.snapshot.as_deref().unwrap(),
        None,
    )
    .unwrap_err();
    assert!(
        error.message.starts_with("RECOVERY_REPORT_"),
        "{}",
        error.message
    );
    assert!(checkpoint_directory.is_dir());
    assert!(!missing.exists());
}

#[test]
fn latest_pointer_to_missing_approval_fails_when_its_checkpoint_directory_is_also_missing() {
    let fixture = approved_fixture();
    let pointer = fixture
        .repo
        .path(&format!(
            ".codeclew/work/{}/latest-run.json",
            fixture.work.id
        ))
        .unwrap();
    let latest: Value = store::read(&pointer, store::MAX_RECORD).unwrap();
    assert_eq!(latest["run"], fixture.review_run);
    let report = fixture
        .repo
        .path(&format!(".codeclew/jobs/{}.json", fixture.review_run))
        .unwrap();
    let directory = fixture
        .repo
        .path(&format!(".codeclew/jobs/{}", fixture.review_run))
        .unwrap();
    fs::remove_file(&report).unwrap();
    fs::remove_dir_all(&directory).unwrap();
    let error = answer_reuse::find(
        &fixture.repo,
        &fixture.work.subject,
        fixture.work.request.clone(),
        fixture.work.snapshot.as_deref().unwrap(),
        None,
    )
    .unwrap_err();
    assert!(
        error.message.starts_with("RECOVERY_REPORT_"),
        "{}",
        error.message
    );
    assert_eq!(
        store::read::<Value>(&pointer, store::MAX_RECORD).unwrap(),
        latest
    );
    assert!(!report.exists());
    assert!(!directory.exists());
}

#[test]
fn missing_work_manifest_fails_complete_history_before_matching_requests() {
    let fixture = approved_fixture();
    let mut other_request = fixture.work.request.clone();
    other_request.question = Some("Which retained source declaration supplies the result?".into());
    let prepared = work::prepare_with_snapshot(
        &fixture.repo,
        fixture.work.subject.clone(),
        other_request,
        fixture.work.snapshot.as_deref(),
    )
    .unwrap();
    let other = work::load(&fixture.repo, prepared["work"].as_str().unwrap()).unwrap();
    assert_ne!(other.id, fixture.work.id);
    let other_author_config = fixture._temp.path().join("second-work-author-config.json");
    let mut author_config: Value = store::read(&fixture.author_config, store::MAX_RECORD).unwrap();
    // Keep the same finite synthetic driver while giving this independent Work
    // its own reservation account; the original fixture admits only one call.
    author_config["budget"]["account"] = json!("second-work-author");
    fs::write(
        &other_author_config,
        serde_json::to_vec(&author_config).unwrap(),
    )
    .unwrap();
    let authored = agent_jobs::run_operation_draft(
        &fixture.repo,
        &other.id,
        Some(&other_author_config),
        false,
        None,
        None,
    )
    .unwrap();
    assert_eq!(authored["status"], "DRAFT", "{authored}");
    let review_config = fixture._temp.path().join("second-work-review-config.json");
    agent_jobs::write_reusable_review_fixture_config(
        &review_config,
        &fixture.author_config,
        "second-work-approve",
    );
    let reviewed = agent_jobs::review_operation_draft(
        &fixture.repo,
        &other.id,
        authored["run"].as_str().unwrap(),
        &review_config,
        None,
    )
    .unwrap();
    assert_eq!(reviewed["status"], "DRAFT_REVIEW_APPROVED", "{reviewed}");
    let run = reviewed["run"].as_str().unwrap();
    let report = fixture
        .repo
        .path(&format!(".codeclew/jobs/{run}.json"))
        .unwrap();
    let checkpoints = fixture.repo.path(&format!(".codeclew/jobs/{run}")).unwrap();
    assert!(report.is_file());
    assert!(checkpoints.is_dir());
    let directory = fixture
        .repo
        .path(&format!(".codeclew/work/{}", other.id))
        .unwrap();
    fs::remove_dir_all(&directory).unwrap();

    // Neither a surviving exact request nor a request with no match may turn
    // incomplete historical Work membership into a complete discovery result.
    let exact_request = fixture.work.request.clone();
    let mut no_match_request = exact_request.clone();
    no_match_request.question = Some("Which unasked source question has no saved answer?".into());
    for request in [exact_request, no_match_request] {
        let error = answer_reuse::find(
            &fixture.repo,
            &fixture.work.subject,
            request,
            fixture.work.snapshot.as_deref().unwrap(),
            None,
        )
        .unwrap_err();
        assert!(
            error.message.starts_with("RECOVERY_REPORT_"),
            "{}",
            error.message
        );
    }
    assert!(report.is_file());
    assert!(checkpoints.is_dir());
    assert!(!directory.exists());
}
