#![cfg(target_os = "macos")]

use super::{
    Fixture, commit, execution_config, find_checkpoint_record, frozen_work, portable_baseline,
    proposal_artifact, proposal_fixture, proposal_submit, read, run_report, work_read, work_run,
};
use serde_json::{Value, json};
use std::{collections::BTreeMap, fs, path::PathBuf};

const EXPANSION_SENTINEL: &str = "REVIEWER_EXPANSION_SOURCE_SENTINEL";
const FOLLOWUP_SENTINEL: &str = "POST_APPROVAL_SOURCE_SENTINEL";

struct Case {
    fixture: Fixture,
    work: String,
    proposal: String,
    proposal_path: PathBuf,
    proposal_bytes: Vec<u8>,
    proposal_reads: String,
    expansion_reference: String,
    followup_reference: String,
    config: Value,
}

fn add_review_fixture_source(source: &std::path::Path, class: &str, sentinel: &str) {
    fs::write(
        source.join(format!("{class}.java")),
        format!("class {class} {{ String helper() {{ return \"{sentinel}\"; }} }}\n"),
    )
    .unwrap();
}

fn unread_source_reference(
    frozen: &Value,
    filename: &str,
    sentinel: &str,
    reads: &Value,
) -> String {
    let sources = frozen["checked"]["services"]["orders"]["sources"]
        .as_object()
        .expect("checked Work retains source records");
    let source_id = sources
        .iter()
        .find(|(_, source)| {
            source["file"]
                .as_str()
                .is_some_and(|file| file.ends_with(filename))
                && source["text"]
                    .as_str()
                    .is_some_and(|text| text.contains(sentinel))
        })
        .map(|(id, _)| id.as_str())
        .expect("requested source record was captured with its in-body sentinel")
        .to_owned();
    let reference = frozen["handles"]
        .as_object()
        .expect("Work retains registered handles")
        .iter()
        .find(|(_, handle)| handle["kind"] == "SOURCE" && handle["id"] == source_id)
        .map(|(reference, _)| reference.clone())
        .expect("captured source has a registered SOURCE reference");
    assert!(
        !receipt_supplies(reads, &reference),
        "selected source must still be unread before reviewer dispatch"
    );
    reference
}

fn receipt_supplies(reads: &Value, reference: &str) -> bool {
    reads["receipts"]
        .as_object()
        .into_iter()
        .flat_map(|receipts| receipts.values())
        .any(|receipt| {
            receipt["supplied"]
                .as_array()
                .into_iter()
                .flatten()
                .any(|supplied| supplied == reference)
        })
}

fn reviewer_config(fixture: &Fixture, proposal_input: &Value, expansion_reference: &str) -> Value {
    let mut config = execution_config(
        fixture,
        json!({"proposal":proposal_input}),
        json!({
            "mode":"reviewer-expand",
            "expansionReference":expansion_reference,
            "expandedSourceSentinel":EXPANSION_SENTINEL
        }),
        None,
    );
    config["authorCalls"] = json!(3);
    config["reviewerCalls"] = json!(4);
    config["expansions"] = json!(1);
    config
}

fn new_case() -> Case {
    let fixture = Fixture::new();
    let source = fixture.service("orders");
    add_review_fixture_source(&source, "SupplementOne", EXPANSION_SENTINEL);
    add_review_fixture_source(&source, "SupplementTwo", FOLLOWUP_SENTINEL);
    commit(&source);

    let (work, proposal_input, _) = proposal_fixture(&fixture);
    let submitted = proposal_submit(&fixture, &work, &proposal_input);
    let proposal = submitted["proposal"].as_str().unwrap().to_owned();
    let artifact = proposal_artifact(&fixture, &submitted);
    let proposal_path = fixture
        .docs
        .join(format!(".codeclew/proposals/{proposal}.json"));
    let proposal_bytes = fs::read(&proposal_path).unwrap();
    let proposal_reads = artifact["readDigest"].as_str().unwrap().to_owned();
    let work_path = fixture
        .docs
        .join(format!(".codeclew/work/{work}/reads.json"));
    let reads = read(&work_path);
    assert_eq!(clew::canonical::hash(&reads).unwrap(), proposal_reads);
    let frozen = frozen_work(&fixture, &work);
    let expansion_reference =
        unread_source_reference(&frozen, "SupplementOne.java", EXPANSION_SENTINEL, &reads);
    let followup_reference =
        unread_source_reference(&frozen, "SupplementTwo.java", FOLLOWUP_SENTINEL, &reads);
    assert_ne!(expansion_reference, followup_reference);
    let config = reviewer_config(&fixture, &proposal_input, &expansion_reference);

    Case {
        fixture,
        work,
        proposal,
        proposal_path,
        proposal_bytes,
        proposal_reads,
        expansion_reference,
        followup_reference,
        config,
    }
}

fn invocation_input(fixture: &Fixture, invocation: &str) -> Value {
    read(
        fixture
            .docs
            .join(format!(".codeclew/job-inputs/{invocation}.json")),
    )
}

fn invocation_result(fixture: &Fixture, invocation: &str) -> Value {
    read(
        fixture
            .docs
            .join(format!(".codeclew/job-results/{invocation}.json")),
    )
}

fn packet_delivers(payload: &Value, reference: &str) -> bool {
    payload["evidence"]["pages"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|page| page["items"].as_array().into_iter().flatten())
        .any(|item| item["reference"] == reference)
        || payload["evidence"]["sourceParts"]
            .as_array()
            .into_iter()
            .flatten()
            .any(|part| part["reference"] == reference)
}

fn directory_snapshot(path: &std::path::Path) -> BTreeMap<String, Vec<u8>> {
    super::tree_file_snapshot(path)
}

#[test]
fn reviewer_expansion_binds_p1_and_preserves_immutable_proposal_p0() {
    let case = new_case();
    let accepted = work_run(&case.fixture, &case.work, &case.config);
    assert_eq!(accepted["status"], "ACCEPTED", "{accepted}");
    let report = run_report(&case.fixture, &accepted);
    assert_eq!(report["proposal"], case.proposal);
    assert_eq!(report["attempts"].as_array().unwrap().len(), 3, "{report}");
    assert_eq!(report["attempts"][0]["role"], "author");
    assert_eq!(report["attempts"][1]["role"], "reviewer");
    assert_eq!(report["attempts"][2]["role"], "reviewer");

    let author_invocation = report["attempts"][0]["invocation"].as_str().unwrap();
    let expansion_invocation = report["attempts"][1]["invocation"].as_str().unwrap();
    let approval_invocation = report["attempts"][2]["invocation"].as_str().unwrap();
    assert_eq!(
        invocation_result(&case.fixture, author_invocation)["result"]["action"],
        "proposal"
    );
    let expansion_result = invocation_result(&case.fixture, expansion_invocation);
    assert_eq!(expansion_result["result"]["action"], "expand");
    assert_eq!(
        expansion_result["result"]["selection"]["references"],
        json!([case.expansion_reference])
    );
    let approval_result = invocation_result(&case.fixture, approval_invocation);
    assert_eq!(approval_result["result"]["action"], "review");
    assert_eq!(approval_result["result"]["review"]["verdict"], "APPROVE");

    let expansion_payload =
        invocation_input(&case.fixture, expansion_invocation)["request"]["payload"].clone();
    let approval_input = invocation_input(&case.fixture, approval_invocation);
    let approval_payload = approval_input["request"]["payload"].clone();
    assert!(!packet_delivers(
        &expansion_payload,
        &case.expansion_reference
    ));
    assert!(packet_delivers(
        &approval_payload,
        &case.expansion_reference
    ));
    assert_ne!(
        expansion_payload["evidenceDigest"],
        approval_payload["evidenceDigest"]
    );

    let artifact = proposal_artifact(&case.fixture, &json!({"proposal":case.proposal.as_str()}));
    assert_eq!(artifact["readDigest"], case.proposal_reads);
    assert_eq!(
        fs::read(&case.proposal_path).unwrap(),
        case.proposal_bytes,
        "reviewer expansion must not rewrite the immutable proposal"
    );

    let run = accepted["run"].as_str().unwrap();
    let terminal = find_checkpoint_record(&case.fixture, run, |checkpoint| {
        checkpoint["phase"] == "TERMINAL" && checkpoint["reviewerInvocation"] == approval_invocation
    });
    let checkpoint = &terminal["checkpoint"];
    let final_reads = read(
        case.fixture
            .docs
            .join(format!(".codeclew/work/{}/reads.json", case.work)),
    );
    let p1 = clew::canonical::hash(&final_reads).unwrap();
    assert_ne!(p1, case.proposal_reads, "reviewer expansion creates P1");
    assert_eq!(checkpoint["readDigest"], p1);
    assert_eq!(checkpoint["proposalReadDigest"], p1);
    assert_eq!(
        checkpoint["proposalId"], case.proposal,
        "P1 remains attached to the original P0 proposal"
    );
    assert!(receipt_supplies(&final_reads, &case.expansion_reference));

    let operation = artifact["narrative"]["operations"][0]["id"]
        .as_str()
        .unwrap();
    let baseline = serde_json::to_value(portable_baseline(&case.fixture)).unwrap();
    let version = &baseline["acceptedVersions"][format!("service:orders/{operation}")];
    assert_eq!(version["proposal"], case.proposal);
    assert_eq!(version["readDigest"], p1);
    assert_eq!(version["invocation"], approval_invocation);
    assert_eq!(
        version["evidenceDigest"],
        approval_payload["evidenceDigest"]
    );
    assert_eq!(
        checkpoint["proposalEvidenceDigest"],
        approval_payload["evidenceDigest"]
    );
    assert_eq!(
        report["review"]["evidenceDigest"],
        approval_payload["evidenceDigest"]
    );
}

#[test]
fn later_work_read_refuses_accepted_republication_without_mutation() {
    let case = new_case();
    let accepted = work_run(&case.fixture, &case.work, &case.config);
    assert_eq!(accepted["status"], "ACCEPTED", "{accepted}");
    let report = run_report(&case.fixture, &accepted);
    assert_eq!(report["attempts"].as_array().unwrap().len(), 3, "{report}");

    let added = work_read(
        &case.fixture,
        &case.work,
        json!({"references":[case.followup_reference]}),
    );
    assert!(
        added["items"].as_array().unwrap().iter().any(|item| {
            item["reference"] == case.followup_reference && item["kind"] == "SOURCE"
        })
    );
    let read_path = case
        .fixture
        .docs
        .join(format!(".codeclew/work/{}/reads.json", case.work));
    let reads_after_supported_work_read = fs::read(&read_path).unwrap();
    let read_value = read(&read_path);
    assert!(receipt_supplies(&read_value, &case.followup_reference));

    let run = accepted["run"].as_str().unwrap();
    let report_path = case.fixture.docs.join(format!(".codeclew/jobs/{run}.json"));
    let latest_run_path = case
        .fixture
        .docs
        .join(format!(".codeclew/work/{}/latest-run.json", case.work));
    let account_path = case.fixture.docs.join("execution/accounts/fixture.json");
    let index_path = case.fixture.docs.join("docs/index.html");
    let jobs_before = directory_snapshot(&case.fixture.docs.join(".codeclew/jobs"));
    let inputs_before = directory_snapshot(&case.fixture.docs.join(".codeclew/job-inputs"));
    let results_before = directory_snapshot(&case.fixture.docs.join(".codeclew/job-results"));
    let accounts_before = directory_snapshot(&case.fixture.docs.join("execution/accounts"));
    let generated_before = directory_snapshot(&case.fixture.docs.join("docs/generated"));
    let report_before = fs::read(&report_path).unwrap();
    let latest_run_before = fs::read(&latest_run_path).unwrap();
    let account_before = fs::read(&account_path).unwrap();
    let index_before = fs::read(&index_path).unwrap();
    let proposal_before = fs::read(&case.proposal_path).unwrap();

    let config_path = case.fixture.input("execution-retry.json", &case.config);
    let (code, error) = case.fixture.run(&[
        "docs",
        "work",
        "run",
        "--work",
        &case.work,
        "--config",
        config_path.to_str().unwrap(),
    ]);
    assert_ne!(
        code, 0,
        "changed accepted reads must not be replay-published"
    );
    let error_text = serde_json::to_string(&error).unwrap();
    assert!(
        error_text.contains("RECOVERY_PUBLICATION_READS_MISMATCH"),
        "expected read-binding refusal, got {error_text}"
    );

    assert_eq!(
        fs::read(&read_path).unwrap(),
        reads_after_supported_work_read
    );
    assert_eq!(fs::read(&report_path).unwrap(), report_before);
    assert_eq!(fs::read(&latest_run_path).unwrap(), latest_run_before);
    assert_eq!(fs::read(&account_path).unwrap(), account_before);
    assert_eq!(fs::read(&index_path).unwrap(), index_before);
    assert_eq!(fs::read(&case.proposal_path).unwrap(), proposal_before);
    assert_eq!(
        directory_snapshot(&case.fixture.docs.join(".codeclew/jobs")),
        jobs_before
    );
    assert_eq!(
        directory_snapshot(&case.fixture.docs.join(".codeclew/job-inputs")),
        inputs_before,
        "rejected retry must not create another paid role input"
    );
    assert_eq!(
        directory_snapshot(&case.fixture.docs.join(".codeclew/job-results")),
        results_before,
        "rejected retry must not create another role result"
    );
    assert_eq!(
        directory_snapshot(&case.fixture.docs.join("execution/accounts")),
        accounts_before
    );
    assert_eq!(
        directory_snapshot(&case.fixture.docs.join("docs/generated")),
        generated_before
    );
}
