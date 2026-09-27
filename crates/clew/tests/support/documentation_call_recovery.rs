#![cfg(target_os = "macos")]
//! Seeded public-run recovery regressions using deterministic fixture calls.
//!
//! These records model interruption boundaries; they are not process-kill
//! experiments and make no provider billing or transport claims.

use super::*;
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
};

struct SeededCall {
    run: String,
    invocation: String,
    reservation: String,
    checkpoint_reference: Value,
    checkpoint_path: PathBuf,
    checkpoint_bytes: Vec<u8>,
    input_path: PathBuf,
    input_bytes: Vec<u8>,
    result_path: PathBuf,
    result_bytes: Option<Vec<u8>>,
    report_path: PathBuf,
    original_report: Value,
    original_attempts: Vec<Value>,
    original_reviewer_invocation: String,
    original_run_reservations: BTreeMap<String, Value>,
    earlier_reservations: BTreeMap<String, Value>,
    input_snapshot: BTreeMap<String, Vec<u8>>,
    result_snapshot: BTreeMap<String, Vec<u8>>,
    published_pointer: Vec<u8>,
    history_after_seed: Value,
    history_html_after_seed: Vec<u8>,
}

fn run_reservations(account: &Value, run: &str) -> BTreeMap<String, Value> {
    account["reservations"]
        .as_object()
        .unwrap()
        .iter()
        .filter(|(_, record)| record["run"] == run)
        .map(|(id, record)| (id.clone(), record.clone()))
        .collect()
}

fn seed_call_checkpoint(
    f: &Fixture,
    work: &str,
    config: &Value,
    phase: &str,
    role: &str,
    pending_status: &str,
    remove_result: bool,
) -> SeededCall {
    let prior_reader_pointer = reader_pointer_bytes(f);
    let accepted = work_run(f, work, config);
    assert_eq!(accepted["status"], "ACCEPTED", "{accepted}");
    let published_pointer =
        reader_pointer_bytes(f).expect("accepted fixture run selects a generated reader bundle");
    let history_after_seed = f.ok(&["docs", "history", "list"]);
    let history_html_after_seed = fs::read(f.docs.join("docs/history.html")).unwrap();
    let run = accepted["run"].as_str().unwrap().to_owned();
    let original_report = run_report(f, &accepted);
    let original_attempts = original_report["attempts"].as_array().unwrap().clone();
    let reviewer_attempt = original_attempts
        .iter()
        .find(|attempt| attempt["role"] == "reviewer")
        .expect("accepted seed includes a reviewer call");
    let original_reviewer_invocation = reviewer_attempt["invocation"].as_str().unwrap().to_owned();

    let selected_record = find_checkpoint_record(f, &run, |checkpoint| {
        checkpoint["phase"] == phase
            && checkpoint["pendingCall"]["status"] == pending_status
            && checkpoint["pendingCall"]["identity"]["role"] == role
    });
    let selected_checkpoint = &selected_record["checkpoint"];
    let invocation = selected_checkpoint["pendingCall"]["identity"]["invocation"]
        .as_str()
        .unwrap()
        .to_owned();
    let reservation = selected_checkpoint["pendingCall"]["identity"]["reservation"]
        .as_str()
        .unwrap()
        .to_owned();
    let attempt_index = original_attempts
        .iter()
        .position(|attempt| attempt["invocation"] == invocation)
        .expect("selected immutable checkpoint belongs to a saved attempt");
    assert_eq!(original_attempts[attempt_index]["role"], role);
    assert_eq!(original_attempts[attempt_index]["reservation"], reservation);
    assert!(
        original_attempts[..attempt_index]
            .iter()
            .all(|attempt| attempt["status"] == "COMPLETED")
    );

    let checkpoint_reference = checkpoint_reference(&run, &selected_record);
    let checkpoint_path = selected_checkpoint_path(f, &run, &checkpoint_reference);
    let checkpoint_bytes = fs::read(&checkpoint_path).unwrap();
    let input_path = f
        .docs
        .join(format!(".codeclew/job-inputs/{invocation}.json"));
    let input_bytes = fs::read(&input_path).expect("dispatched call has its immutable input");
    let input: Value = serde_json::from_slice(&input_bytes).unwrap();
    assert_eq!(input["identity"]["invocation"], invocation);
    assert_eq!(input["identity"]["reservation"], reservation);
    assert_eq!(input["identity"]["role"], role);
    assert_eq!(
        input["identity"]["inputDigest"],
        original_attempts[attempt_index]["inputDigest"]
    );

    let result_path = f
        .docs
        .join(format!(".codeclew/job-results/{invocation}.json"));
    let result_bytes = fs::read(&result_path).ok();
    if pending_status == "RESULT_SAVED" {
        let saved = result_bytes
            .as_deref()
            .expect("RESULT_SAVED checkpoint has a durable result");
        let result: Value = serde_json::from_slice(saved).unwrap();
        assert_eq!(result["identity"]["invocation"], invocation);
        assert_eq!(result["identity"]["reservation"], reservation);
        assert_eq!(result["identity"]["role"], role);
        assert_eq!(result["transportStatus"], "VALIDATED_REPLY");
        assert!(!remove_result);
    } else {
        assert_eq!(pending_status, "DISPATCHED");
        assert!(result_bytes.is_some(), "successful seed made this result");
        assert!(
            remove_result,
            "only the absent-result case removes its result"
        );
        fs::remove_file(&result_path).unwrap();
        assert!(!result_path.exists());
    }

    let report_path = f.docs.join(format!(".codeclew/jobs/{run}.json"));
    let mut seeded_report = original_report.clone();
    let mut replay_attempts = original_attempts[..=attempt_index].to_vec();
    replay_attempts[attempt_index]["status"] = json!("DISPATCHED");
    replay_attempts[attempt_index]["failure"] = Value::Null;
    if pending_status == "DISPATCHED" {
        replay_attempts[attempt_index]["usage"] = Value::Null;
        replay_attempts[attempt_index]["resultDigest"] = Value::Null;
        replay_attempts[attempt_index]["capturedStdoutBytes"] = json!(0);
        replay_attempts[attempt_index]["capturedStderrBytes"] = json!(0);
    }
    seeded_report["status"] = json!(if phase == "AUTHOR" {
        "PREPARED"
    } else {
        "CHECKED"
    });
    seeded_report["attempts"] = Value::Array(replay_attempts);
    if phase == "AUTHOR" {
        seeded_report["proposal"] = Value::Null;
    }
    seeded_report["review"] = Value::Null;
    seeded_report["publication"] = Value::Null;
    seeded_report["gap"] = Value::Null;
    seeded_report["accounting"] = Value::Null;
    seeded_report["checkpoint"] = checkpoint_reference.clone();
    fs::write(&report_path, serde_json::to_vec(&seeded_report).unwrap()).unwrap();

    let account_path = f.docs.join("execution/accounts/fixture.json");
    let original_account = read(&account_path);
    let original_run_reservations = run_reservations(&original_account, &run);
    assert!(!original_run_reservations.is_empty());
    let earlier_ids: BTreeSet<_> = original_attempts[..attempt_index]
        .iter()
        .map(|attempt| attempt["reservation"].as_str().unwrap().to_owned())
        .collect();
    let earlier_reservations: BTreeMap<_, _> = earlier_ids
        .iter()
        .map(|id| (id.clone(), original_run_reservations[id].clone()))
        .collect();
    let mut seeded_account = original_account.clone();
    let reservations = seeded_account["reservations"].as_object_mut().unwrap();
    for (id, record) in reservations.iter_mut() {
        if record["run"] != run || earlier_ids.contains(id) {
            continue;
        }
        record["charged"] = record["maximum"].clone();
        record["actual"] = Value::Null;
        if id == &reservation {
            record["status"] = json!("DISPATCHED");
        } else {
            record["status"] = json!("RESERVED");
        }
    }
    assert_eq!(reservations[&reservation]["role"], role);
    fs::write(&account_path, serde_json::to_vec(&seeded_account).unwrap()).unwrap();

    restore_reader_pointer(f, prior_reader_pointer.as_deref());
    assert_eq!(reader_pointer_bytes(f), prior_reader_pointer);
    let input_snapshot = tree_file_snapshot(&f.docs.join(".codeclew/job-inputs"));
    let result_snapshot = tree_file_snapshot(&f.docs.join(".codeclew/job-results"));
    SeededCall {
        run,
        invocation,
        reservation,
        checkpoint_reference,
        checkpoint_path,
        checkpoint_bytes,
        input_path,
        input_bytes,
        result_path,
        result_bytes,
        report_path,
        original_report,
        original_attempts,
        original_reviewer_invocation,
        original_run_reservations,
        earlier_reservations,
        input_snapshot,
        result_snapshot,
        published_pointer,
        history_after_seed,
        history_html_after_seed,
    }
}

fn assert_original_files_retained(
    f: &Fixture,
    directory: &str,
    before: &BTreeMap<String, Vec<u8>>,
) {
    let after = tree_file_snapshot(&f.docs.join(directory));
    for (path, bytes) in before {
        assert_eq!(
            after.get(path),
            Some(bytes),
            "immutable call record changed: {path}"
        );
    }
}

fn released_history_ids(history: &Value) -> BTreeSet<String> {
    history["items"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|row| row["released"] == true)
        .map(|row| row["id"].as_str().unwrap().to_owned())
        .collect()
}

fn selected_bundle(f: &Fixture) -> String {
    let index = fs::read_to_string(f.docs.join("docs/index.html")).unwrap();
    index
        .lines()
        .next()
        .unwrap()
        .strip_prefix("<!-- codeclew-bundle ")
        .unwrap()
        .strip_suffix(" -->")
        .unwrap()
        .to_owned()
}

fn assert_no_new_released_history(f: &Fixture, seed: &SeededCall) {
    let current_history = f.ok(&["docs", "history", "list"]);
    assert_eq!(
        released_history_ids(&current_history),
        released_history_ids(&seed.history_after_seed),
        "recovery added a released history entry"
    );
}

fn accepted_version_for_invocation<'a>(bindings: &'a Value, invocation: &str) -> &'a Value {
    bindings["acceptedVersions"]
        .as_object()
        .unwrap()
        .values()
        .find(|version| version["invocation"] == invocation)
        .expect("published AcceptedVersion binds the reviewer invocation")
}

fn assert_new_reviewer_publication(
    f: &Fixture,
    report: &Value,
    seed: &SeededCall,
    reviewer_attempt: &Value,
) {
    let original_bundle = seed.original_report["publication"]["bundle"]
        .as_str()
        .unwrap();
    let new_bundle = report["publication"]["bundle"].as_str().unwrap();
    assert_ne!(new_bundle, original_bundle);
    assert_eq!(selected_bundle(f), new_bundle);
    assert_no_new_released_history(f, seed);

    let original_operations =
        read(f.bundle(original_bundle, "services/orders.json"))["operations"].clone();
    let new_operations = read(f.bundle(new_bundle, "services/orders.json"))["operations"].clone();
    assert_eq!(
        new_operations, original_operations,
        "recovery changed the accepted operation's semantic content"
    );
    let original_bindings = read(f.bundle(original_bundle, "bindings.json"));
    let new_bindings = read(f.bundle(new_bundle, "bindings.json"));
    let original_version =
        accepted_version_for_invocation(&original_bindings, &seed.original_reviewer_invocation);
    let new_version = accepted_version_for_invocation(
        &new_bindings,
        reviewer_attempt["invocation"].as_str().unwrap(),
    );
    assert_eq!(new_version["proposal"], report["proposal"]);
    assert_eq!(
        new_version["operationDigest"], original_version["operationDigest"],
        "new reviewer binding changed the accepted operation"
    );
}

fn assert_same_publication(f: &Fixture, report: &Value, seed: &SeededCall) {
    assert_eq!(report["publication"], seed.original_report["publication"]);
    assert_eq!(
        reader_pointer_bytes(f),
        Some(seed.published_pointer.clone())
    );
    assert_eq!(
        selected_bundle(f),
        seed.original_report["publication"]["bundle"]
    );
    assert_eq!(f.ok(&["docs", "history", "list"]), seed.history_after_seed);
    assert_eq!(
        fs::read(f.docs.join("docs/history.html")).unwrap(),
        seed.history_html_after_seed
    );
}

fn assert_report_accounting_matches_ledger(
    f: &Fixture,
    report: &Value,
    seed: &SeededCall,
) -> BTreeMap<String, Value> {
    let account = read(f.docs.join("execution/accounts/fixture.json"));
    let final_reservations = run_reservations(&account, &seed.run);
    assert_eq!(
        final_reservations.keys().collect::<BTreeSet<_>>(),
        seed.original_run_reservations
            .keys()
            .collect::<BTreeSet<_>>(),
        "recovery allocated or discarded a finite reservation"
    );
    let report_reservations: BTreeMap<_, _> = report["accounting"]
        .as_array()
        .expect("accepted report records final accounting")
        .iter()
        .map(|row| {
            (
                row["reservation"].as_str().unwrap().to_owned(),
                row["record"].clone(),
            )
        })
        .collect();
    assert_eq!(report_reservations, final_reservations);
    final_reservations
}

fn assert_attempt_result_matches_saved(seed: &SeededCall, attempt: &Value) {
    let saved = seed
        .result_bytes
        .as_deref()
        .expect("saved-result replay has a durable preimage");
    let record: Value = serde_json::from_slice(saved).unwrap();
    assert_eq!(attempt["invocation"], seed.invocation);
    assert_eq!(attempt["reservation"], seed.reservation);
    assert_eq!(attempt["role"], record["identity"]["role"]);
    assert_eq!(attempt["usage"], record["usage"]);
    assert_eq!(attempt["resultDigest"], record["resultDigest"]);
    assert_eq!(fs::read(&seed.input_path).unwrap(), seed.input_bytes);
    assert_eq!(fs::read(&seed.result_path).unwrap(), saved);
    assert_eq!(
        fs::read(&seed.checkpoint_path).unwrap(),
        seed.checkpoint_bytes
    );
}

#[test]
fn author_saved_result_reconciles_without_redispatching_author() {
    let f = Fixture::new();
    f.service("orders");
    let (work, _, _) = proposal_fixture(&f);
    let config = execution_config(&f, json!({}), json!({}), None);
    let seed = seed_call_checkpoint(
        &f,
        &work,
        &config,
        "AUTHOR",
        "author",
        "RESULT_SAVED",
        false,
    );
    assert_eq!(seed.checkpoint_reference["run"], seed.run);
    assert_eq!(read(&seed.report_path)["status"], "PREPARED");

    let resumed = work_run(&f, &work, &config);
    assert_eq!(resumed["run"], seed.run);
    assert_eq!(resumed["status"], "ACCEPTED", "{resumed}");
    let report = run_report(&f, &resumed);
    let attempts = report["attempts"].as_array().unwrap();
    assert_eq!(attempts.len(), 2, "{report}");
    assert_eq!(attempts[0], seed.original_attempts[0]);
    assert_eq!(attempts[0]["role"], "author");
    assert_eq!(attempts[0]["status"], "COMPLETED");
    assert_attempt_result_matches_saved(&seed, &attempts[0]);
    assert_eq!(attempts[1]["role"], "reviewer");
    assert_eq!(attempts[1]["status"], "COMPLETED");
    assert_ne!(attempts[1]["invocation"], seed.original_reviewer_invocation);
    assert_eq!(
        attempts[1]["reservation"],
        seed.original_attempts
            .iter()
            .find(|attempt| attempt["role"] == "reviewer")
            .unwrap()["reservation"]
    );
    assert_eq!(report["proposal"], seed.original_report["proposal"]);
    let final_reservations = assert_report_accounting_matches_ledger(&f, &report, &seed);
    assert_eq!(
        final_reservations[&seed.reservation]["status"],
        "RECONCILED"
    );
    assert_original_files_retained(&f, ".codeclew/job-inputs", &seed.input_snapshot);
    assert_original_files_retained(&f, ".codeclew/job-results", &seed.result_snapshot);
    assert_new_reviewer_publication(&f, &report, &seed, &attempts[1]);
}

#[test]
fn author_uncertainty_adaptation_replays_saved_result_without_redispatch() {
    let f = Fixture::new();
    f.service("orders");
    let (work, _, _) = proposal_fixture(&f);
    let config = execution_config(
        &f,
        json!({"mode":"ordinary-uncertainty-adapt"}),
        json!({}),
        None,
    );
    let seed = seed_call_checkpoint(
        &f,
        &work,
        &config,
        "AUTHOR",
        "author",
        "RESULT_SAVED",
        false,
    );
    let expected_receipt = seed.original_attempts[0]["uncertaintyAdaptation"].clone();
    assert!(expected_receipt.is_object());

    let mut checkpointed_report = read(&seed.report_path);
    let attempt = checkpointed_report["attempts"][0].as_object_mut().unwrap();
    attempt.remove("uncertaintyAdaptation");
    attempt.remove("adaptedProposal");
    fs::write(
        &seed.report_path,
        serde_json::to_vec(&checkpointed_report).unwrap(),
    )
    .unwrap();

    let resumed = work_run(&f, &work, &config);
    assert_eq!(resumed["run"], seed.run);
    assert_eq!(resumed["status"], "ACCEPTED", "{resumed}");
    let report = run_report(&f, &resumed);
    let attempts = report["attempts"].as_array().unwrap();
    assert_eq!(attempts.len(), 2, "{report}");
    assert_eq!(attempts[0]["role"], "author");
    assert_eq!(attempts[0]["invocation"], seed.invocation);
    assert_eq!(attempts[0]["status"], "COMPLETED");
    assert_eq!(attempts[0]["uncertaintyAdaptation"], expected_receipt);
    assert_eq!(
        attempts[0]["adaptedProposal"],
        seed.original_attempts[0]["adaptedProposal"]
    );
    assert_eq!(attempts[1]["role"], "reviewer");
    assert_attempt_result_matches_saved(&seed, &attempts[0]);
    assert_original_files_retained(&f, ".codeclew/job-inputs", &seed.input_snapshot);
    assert_original_files_retained(&f, ".codeclew/job-results", &seed.result_snapshot);
}

#[test]
fn reviewer_saved_result_reconciles_without_redispatching_reviewer() {
    let f = Fixture::new();
    f.service("orders");
    let (work, _, _) = proposal_fixture(&f);
    let config = execution_config(&f, json!({}), json!({}), None);
    let seed = seed_call_checkpoint(
        &f,
        &work,
        &config,
        "REVIEWER",
        "reviewer",
        "RESULT_SAVED",
        false,
    );
    assert_eq!(seed.checkpoint_reference["run"], seed.run);
    assert_eq!(read(&seed.report_path)["status"], "CHECKED");

    let resumed = work_run(&f, &work, &config);
    assert_eq!(resumed["run"], seed.run);
    assert_eq!(resumed["status"], "ACCEPTED", "{resumed}");
    let report = run_report(&f, &resumed);
    assert_eq!(report["attempts"], seed.original_report["attempts"]);
    assert_same_publication(&f, &report, &seed);
    assert_eq!(report["proposal"], seed.original_report["proposal"]);
    assert_eq!(report["review"], seed.original_report["review"]);
    assert_attempt_result_matches_saved(
        &seed,
        report["attempts"]
            .as_array()
            .unwrap()
            .iter()
            .find(|attempt| attempt["invocation"] == seed.invocation)
            .unwrap(),
    );
    let final_reservations = assert_report_accounting_matches_ledger(&f, &report, &seed);
    for (id, earlier) in &seed.earlier_reservations {
        assert_eq!(
            &final_reservations[id], earlier,
            "prior author charge changed"
        );
    }
    assert_eq!(final_reservations, seed.original_run_reservations);
    assert_eq!(
        tree_file_snapshot(&f.docs.join(".codeclew/job-inputs")),
        seed.input_snapshot
    );
    assert_eq!(
        tree_file_snapshot(&f.docs.join(".codeclew/job-results")),
        seed.result_snapshot
    );
}

#[test]
fn dispatched_author_without_saved_result_retains_maximum_and_uses_original_slots() {
    let f = Fixture::new();
    f.service("orders");
    let (work, _, _) = proposal_fixture(&f);
    let config = execution_config(&f, json!({}), json!({}), None);
    let seed = seed_call_checkpoint(&f, &work, &config, "AUTHOR", "author", "DISPATCHED", true);
    assert!(seed.result_bytes.is_some());
    assert!(!seed.result_path.exists());
    assert_eq!(read(&seed.report_path)["status"], "PREPARED");

    let resumed = work_run(&f, &work, &config);
    assert_eq!(resumed["run"], seed.run);
    assert_eq!(resumed["status"], "ACCEPTED", "{resumed}");
    let report = run_report(&f, &resumed);
    let attempts = report["attempts"].as_array().unwrap();
    assert_eq!(attempts.len(), 3, "{report}");
    assert_eq!(attempts[0]["role"], "author");
    assert_eq!(attempts[0]["invocation"], seed.invocation);
    assert_eq!(attempts[0]["reservation"], seed.reservation);
    assert_eq!(attempts[0]["status"], "INTERRUPTED_MAXIMUM_RETAINED");
    assert_eq!(
        attempts[0]["failure"],
        "RECOVERY_INTERRUPTED_NO_DURABLE_RESULT"
    );
    assert!(attempts[0]["usage"].is_null());
    assert!(attempts[0]["resultDigest"].is_null());
    assert_eq!(attempts[1]["role"], "author");
    assert_eq!(attempts[1]["status"], "COMPLETED");
    assert_ne!(attempts[1]["invocation"], seed.invocation);
    assert_eq!(attempts[2]["role"], "reviewer");
    assert_eq!(attempts[2]["status"], "COMPLETED");
    let author_slots: Vec<_> = seed
        .original_run_reservations
        .iter()
        .filter(|(_, record)| record["role"] == "author")
        .map(|(id, _)| id.clone())
        .collect();
    assert_eq!(author_slots.len(), 2);
    let next_author_slot = author_slots
        .iter()
        .find(|id| *id != &seed.reservation)
        .unwrap();
    assert_eq!(attempts[1]["reservation"], *next_author_slot);
    assert!(
        seed.original_run_reservations
            .iter()
            .filter(|(_, record)| record["role"] == "reviewer")
            .any(|(id, _)| attempts[2]["reservation"] == *id)
    );
    assert_ne!(attempts[1]["invocation"], attempts[2]["invocation"]);

    assert_eq!(fs::read(&seed.input_path).unwrap(), seed.input_bytes);
    assert!(!seed.result_path.exists());
    assert_original_files_retained(&f, ".codeclew/job-inputs", &seed.input_snapshot);
    assert_original_files_retained(&f, ".codeclew/job-results", &seed.result_snapshot);
    let final_reservations = assert_report_accounting_matches_ledger(&f, &report, &seed);
    let unresolved = &final_reservations[&seed.reservation];
    assert_eq!(unresolved["status"], "UNRECONCILED_MAXIMUM_RETAINED");
    assert_eq!(unresolved["charged"], unresolved["maximum"]);
    assert!(unresolved["actual"].is_null());
    for attempt in &attempts[1..] {
        assert_eq!(
            final_reservations[attempt["reservation"].as_str().unwrap()]["status"],
            "RECONCILED"
        );
    }
    assert_new_reviewer_publication(&f, &report, &seed, &attempts[2]);
}
