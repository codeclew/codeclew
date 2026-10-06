//! Opted-in host dispatch using local, deterministic compatible drivers.
#![cfg(target_os = "macos")]

use super::{
    agent_jobs::{self, operation_draft_review},
    model_ids,
    store::{self, Repository},
};
use serde_json::{Value, json};
use std::{
    fs,
    io::{Read, Write},
    net::TcpListener,
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    thread,
};

// The driver sandbox denies writes. A loopback-only acknowledged counter counts
// actual dispatches without changing any production recovery objects.
struct DispatchCounter {
    port: u16,
    calls: Arc<AtomicUsize>,
    stop: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<()>>,
}

impl DispatchCounter {
    fn new() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        listener.set_nonblocking(true).unwrap();
        let calls = Arc::new(AtomicUsize::new(0));
        let stop = Arc::new(AtomicBool::new(false));
        let (count, stopped) = (calls.clone(), stop.clone());
        let worker = thread::spawn(move || {
            while !stopped.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        stream
                            .set_read_timeout(Some(std::time::Duration::from_secs(2)))
                            .unwrap();
                        let mut byte = [0];
                        stream.read_exact(&mut byte).unwrap();
                        assert_eq!(byte, [b'D']);
                        count.fetch_add(1, Ordering::SeqCst);
                        stream.write_all(b"A").unwrap();
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(std::time::Duration::from_millis(2))
                    }
                    Err(error) => panic!("counter failed: {error}"),
                }
            }
        });
        Self {
            port,
            calls,
            stop,
            worker: Some(worker),
        }
    }
    fn count(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

impl Drop for DispatchCounter {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        self.worker.take().unwrap().join().unwrap();
    }
}

const SERIALIZER: &str = r#"
require "digest"
require "socket"
carrier = JSON.parse(STDIN.read)
abort "carrier version" unless ["codeclew-model-ids/1.0", "codeclew-model-ids/1.1"].include?(FIXTURE_VERSION) && carrier.fetch("schema") == FIXTURE_VERSION
canonical_job = carrier.fetch("canonicalJob")
def sorted(v)
  case v
  when Hash then v.keys.sort.to_h { |k| [k, sorted(v[k])] }
  when Array then v.map { |x| sorted(x) }
  else v
  end
end
def content_digest(v)
  "sha256:" + Digest::SHA256.hexdigest(JSON.generate(sorted(v)))
end
# Existing canonical checks precede reading the prepared model form.
abort "canonical job schema" unless canonical_job.fetch("schema") == "codeclew-documentation-agent-job/1.0"
canonical_packet = canonical_job.fetch("payload").fetch("packet")
unsigned_packet = canonical_packet.reject { |k,_| k == "packetDigest" }
abort "packet content binding differs" unless content_digest(unsigned_packet) == canonical_packet.fetch("packetDigest")
prepared = carrier.fetch("preparedModel")
abort "prepared version" unless prepared.fetch("version") == FIXTURE_VERSION && prepared.fetch("map").fetch("version") == FIXTURE_VERSION
abort "canonical carrier binding" unless prepared.fetch("canonicalJobDigest") == content_digest(canonical_job)
model_payload = prepared.fetch("modelPayload")
output_schema = prepared.fetch("outputSchema")
abort "model payload binding" unless content_digest(model_payload) == prepared.fetch("modelPayloadDigest")
abort "strict schema binding" unless content_digest(output_schema) == prepared.fetch("outputSchemaDigest") && output_schema == model_payload.fetch("outputSchema")
def check_schema(v)
  if v.is_a?(Hash)
    if v["const"].is_a?(String) && v["pattern"].is_a?(String)
      abort "strict schema const contradicts pattern" unless Regexp.new(v["pattern"]).match?(v["const"])
    end
    v.each_value { |x| check_schema(x) }
  elsif v.is_a?(Array)
    v.each { |x| check_schema(x) }
  end
end
check_schema(output_schema)
abort "map metadata entered payload" if model_payload.key?("map") || model_payload.key?("canonicalJob")
abort "projection omitted" if model_payload.fetch("packet").fetch("packetDigest") == canonical_packet.fetch("packetDigest")
scope = prepared.fetch("scope")
abort "role scope" unless scope.fetch("work") == canonical_job.fetch("work") && scope.fetch("role") == canonical_job.fetch("role")
socket = TCPSocket.new("127.0.0.1", FIXTURE_PORT)
socket.write("D")
abort "dispatch acknowledgement" unless socket.read(1) == "A"
socket.close
at_exit { exit 7 } if FIXTURE_MODE == "adapter-failure"
# The deterministic model receives only the prepared payload and strict schema.
model_job = canonical_job.merge("payload" => model_payload)
def alias_fixture_reply(reply)
  answer = reply.fetch("result")["answer"]
  if answer && ["unknown", "wrong-scope"].include?(FIXTURE_MODE)
    original = answer.fetch("packetDigest")
    answer["packetDigest"] = FIXTURE_MODE == "unknown" ? "d999999_" + original.split("_").last : original.sub(/_[^_]+$/, "_ffffffffffffffff")
  end
  JSON.generate(reply)
end
"#;

fn opt_in(path: &Path, role: &str, counter: &DispatchCounter, mode: &str) -> Value {
    opt_in_with_version(path, role, counter, mode, model_ids::VERSION)
}

fn opt_in_with_version(
    path: &Path,
    role: &str,
    counter: &DispatchCounter,
    mode: &str,
    version: &str,
) -> Value {
    let mut config: Value = store::read(path, store::MAX_RECORD).unwrap();
    let original = config[role]["command"][4].as_str().unwrap();
    let model = original
        .replace("request = JSON.parse(STDIN.read)", "request = model_job")
        .replace("r = JSON.parse(STDIN.read)", "r = model_job")
        .replace("puts JSON.generate(", "puts alias_fixture_reply(");
    assert!(!model.contains("JSON.parse(STDIN.read)"));
    config[role]["command"][4] = json!(format!(
        "FIXTURE_PORT = {}\nFIXTURE_MODE = {:?}\nFIXTURE_VERSION = {:?}\n{}\n{}",
        counter.port, mode, version, SERIALIZER, model
    ));
    config[role]["modelRepresentation"] = json!(version);
    config[role]["network"] = json!(true);
    if role == "reviewer" {
        // This synthetic grouped delivery contains a large source fragment and
        // the full saved author contract. Admit the complete carrier explicitly,
        // including canonical input and the host-only map, for all three calls.
        config[role]["cap"]["maximum"]["inputTokens"] = json!(1_000_000);
        config["budget"]["ceiling"]["inputTokens"] = json!(4_000_000);
        config["budget"]["stopLoss"]["inputTokens"] = json!(3_500_000);
    }
    fs::write(path, serde_json::to_vec(&config).unwrap()).unwrap();
    config
}

fn read(repo: &Repository, directory: &str, invocation: &str) -> Value {
    store::read(
        &repo
            .path(&format!(".codeclew/{directory}/{invocation}.json"))
            .unwrap(),
        4 * 1024 * 1024,
    )
    .unwrap()
}

fn prepared(repo: &Repository, attempt: &Value) -> (Value, model_ids::Prepared) {
    let invocation = attempt["invocation"].as_str().unwrap();
    let canonical = read(repo, "job-inputs", invocation)["request"].clone();
    let model = read(repo, "job-model-inputs", invocation);
    assert_eq!(model["carrier"]["canonicalJob"], canonical);
    assert_eq!(model["carrier"]["preparedModel"], model["prepared"]);
    let prepared: model_ids::Prepared = serde_json::from_value(model["prepared"].clone()).unwrap();
    model_ids::validate(&canonical, &prepared).unwrap();
    (canonical, prepared)
}

fn checked_native_presentation(delivery: &Value) -> Value {
    let presentation = super::job_context::present(
        delivery["pages"].as_array().unwrap(),
        delivery["sourceParts"].as_array().unwrap(),
    );
    assert_eq!(presentation, delivery["presentation"]);
    presentation
}

// Protected source and query leaves stay exact in raw arrays or in the checked
// native presentation when compact delivery omits duplicate raw siblings.
fn protected(original: &Value, projected: &Value) {
    match original {
        Value::Object(object) => {
            for (key, value) in object {
                if matches!(key.as_str(), "pages" | "sourceParts") && projected.get(key).is_none() {
                    let presentation = checked_native_presentation(original);
                    protected(&presentation[key], &projected["presentation"][key]);
                    continue;
                }
                if matches!(
                    key.as_str(),
                    "text"
                        | "tokens"
                        | "symbol"
                        | "symbolIdentity"
                        | "technicalName"
                        | "identity"
                        | "file"
                        | "query"
                        | "symbols"
                        | "references"
                        | "sourceReferences"
                        | "dependencyReferences"
                ) {
                    assert_eq!(value, &projected[key], "protected field {key}");
                } else {
                    protected(value, &projected[key]);
                }
            }
        }
        Value::Array(values) => {
            for (index, value) in values.iter().enumerate() {
                protected(value, &projected[index]);
            }
        }
        _ => {}
    }
}

// Test-only inverse over exact alias leaves permits whole-delivery equality:
// offsets, source receipts, evidence labels and continuation cursors are checked
// alongside the separate protected-text assertions. This never rewrites stores.
fn canonical_alias_leaves(value: &Value, map: &model_ids::AliasMap) -> Value {
    match value {
        Value::String(text) => map
            .entries
            .iter()
            .find(|entry| entry.alias == *text)
            .map_or_else(|| value.clone(), |entry| json!(entry.canonical)),
        Value::Array(values) => json!(
            values
                .iter()
                .map(|value| canonical_alias_leaves(value, map))
                .collect::<Vec<_>>()
        ),
        Value::Object(object) => Value::Object(
            object
                .iter()
                .map(|(key, value)| (key.clone(), canonical_alias_leaves(value, map)))
                .collect(),
        ),
        _ => value.clone(),
    }
}

fn assert_model_delivery(job: &Value, prepared: &model_ids::Prepared) {
    for (canonical, model) in [
        (
            &job["payload"]["packet"]["contextDelivery"],
            &prepared.model_payload["packet"]["contextDelivery"],
        ),
        (
            &job["payload"]["reviewContext"],
            &prepared.model_payload["reviewContext"],
        ),
    ] {
        if !canonical.is_object() {
            continue;
        }
        let mut expected = canonical.clone();
        if prepared.version == model_ids::COMPACT_VERSION {
            checked_native_presentation(canonical);
            for key in ["pages", "sourceParts"] {
                assert!(model.get(key).is_none(), "compact raw {key} must be absent");
                expected.as_object_mut().unwrap().remove(key);
            }
        } else {
            assert!(model["pages"].is_array() && model["sourceParts"].is_array());
        }
        assert_eq!(canonical_alias_leaves(model, &prepared.map), expected);
    }
    // The compact scope excludes the archived author contract.
    if job["payload"]["savedAuthorContract"]["packet"]["contextDelivery"].is_object() {
        assert!(
            prepared.model_payload["savedAuthorContract"]["packet"]["contextDelivery"]["pages"]
                .is_array()
        );
        assert!(prepared.model_payload["savedAuthorContract"]["packet"]["contextDelivery"]["sourceParts"].is_array());
    }
}

#[test]
fn model_ids_host_grouped_expansion_and_independent_review_save_canonical_results() {
    grouped_expansion_and_review(model_ids::VERSION);
}

#[test]
fn model_ids_compact_host_grouped_expansion_and_independent_review_save_canonical_results() {
    grouped_expansion_and_review(model_ids::COMPACT_VERSION);
}

fn grouped_expansion_and_review(version: &str) {
    let (_temp, repo, work, author_path, _) = agent_jobs::model_ids_grouped_author_fixture_setup();
    let counter = DispatchCounter::new();
    opt_in_with_version(&author_path, "author", &counter, "valid", version);
    let authored =
        agent_jobs::run_operation_draft(&repo, &work.id, Some(&author_path), false, None, None)
            .unwrap();
    assert_eq!(authored["status"], "DRAFT", "{authored}");
    let attempts = authored["attempts"].as_array().unwrap();
    assert_eq!(attempts.len(), 2);
    assert_eq!(counter.count(), 2);
    let (first_job, first) = prepared(&repo, &attempts[0]);
    let (second_job, second) = prepared(&repo, &attempts[1]);
    assert_eq!(first.version, version);
    assert_eq!(second.version, version);
    assert_model_delivery(&first_job, &first);
    assert_model_delivery(&second_job, &second);
    assert!(second.map.entries.len() > first.map.entries.len());
    assert_eq!(
        &second.map.entries[..first.map.entries.len()],
        first.map.entries.as_slice()
    );
    assert!(
        second_job["payload"]["packet"]["contextDelivery"]["pages"]
            .as_array()
            .unwrap()
            .len()
            > 3
    );
    assert!(
        second_job["payload"]["packet"]["contextDelivery"]["sourceParts"]
            .as_array()
            .unwrap()
            .len()
            > 1
    );
    protected(&first_job["payload"], &first.model_payload);
    protected(&second_job["payload"], &second.model_payload);
    let final_invocation = attempts[1]["invocation"].as_str().unwrap();
    let raw = read(&repo, "job-model-wire-results", final_invocation);
    let saved = read(&repo, "job-results", final_invocation);
    assert_eq!(
        raw["output"]["result"]["answer"]["packetDigest"],
        second.model_payload["packet"]["packetDigest"]
    );
    assert_eq!(
        saved["result"]["answer"]["packetDigest"],
        second_job["payload"]["packet"]["packetDigest"]
    );
    assert_eq!(
        saved["modelBinding"]["rawRecordDigest"],
        raw["recordDigest"]
    );

    let reviewer_path = author_path.with_file_name("model-id-review.json");
    agent_jobs::write_model_ids_grouped_review_fixture_config(&reviewer_path, &author_path);
    opt_in_with_version(&reviewer_path, "reviewer", &counter, "valid", version);
    let reviewed = agent_jobs::review_operation_draft(
        &repo,
        &work.id,
        authored["run"].as_str().unwrap(),
        &reviewer_path,
        None,
    )
    .unwrap();
    assert_eq!(reviewed["status"], "DRAFT_REVIEW_APPROVED", "{reviewed}");
    let review_attempts = reviewed["attempts"].as_array().unwrap();
    assert_eq!(review_attempts.len(), 2);
    assert_eq!(counter.count(), 4);
    let (review_initial_job, initial_review) = prepared(&repo, &review_attempts[0]);
    let (review_final_job, final_review) = prepared(&repo, &review_attempts[1]);
    assert_eq!(initial_review.version, version);
    assert_eq!(final_review.version, version);
    assert_model_delivery(&review_initial_job, &initial_review);
    assert_model_delivery(&review_final_job, &final_review);
    assert!(
        review_initial_job["payload"]["reviewContext"]["pages"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert!(
        !review_final_job["payload"]["reviewContext"]["pages"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        &final_review.map.entries[..initial_review.map.entries.len()],
        initial_review.map.entries.as_slice()
    );
    assert_ne!(first.map.scope_tag, initial_review.map.scope_tag);
    let packet_digest = second_job["payload"]["packet"]["packetDigest"]
        .as_str()
        .unwrap();
    let author_alias = second
        .map
        .entries
        .iter()
        .find(|entry| entry.domain == model_ids::Domain::Digest && entry.canonical == packet_digest)
        .unwrap();
    let reviewer_alias = final_review
        .map
        .entries
        .iter()
        .find(|entry| entry.domain == model_ids::Domain::Digest && entry.canonical == packet_digest)
        .unwrap();
    assert_ne!(author_alias.alias, reviewer_alias.alias);
    protected(&review_final_job["payload"], &final_review.model_payload);
    let approved = operation_draft_review::load_approved_answer(
        &repo,
        &work,
        reviewed["run"].as_str().unwrap(),
    )
    .unwrap();
    assert_eq!(
        approved.answer["packetDigest"],
        second_job["payload"]["packet"]["packetDigest"]
    );
    assert_eq!(approved.review["work"], work.id);
    assert_eq!(approved.review["sourceRun"], authored["run"]);
    assert_eq!(approved.review["sourceInvocation"], final_invocation);
    let final_review_invocation = review_attempts[1]["invocation"].as_str().unwrap();
    let review_saved = read(&repo, "job-results", final_review_invocation);
    assert_eq!(review_saved["result"]["review"], approved.review);
    let resumed = agent_jobs::review_operation_draft(
        &repo,
        &work.id,
        authored["run"].as_str().unwrap(),
        &reviewer_path,
        None,
    )
    .unwrap();
    assert_eq!(resumed["run"], reviewed["run"]);
    assert_eq!(resumed["status"], "DRAFT_REVIEW_APPROVED");
    assert_eq!(counter.count(), 4);
    // Current gated chains establish recovery compatibility; they are not
    // historical e729cd2 goldens. Exercise the production retained-record checks.
    for attempt in attempts.iter().chain(review_attempts.iter()) {
        let invocation = attempt["invocation"].as_str().unwrap();
        let validated =
            agent_jobs::validate_frozen_model_records_for_test(&repo, invocation).unwrap();
        assert_eq!(validated["version"], version);
        assert_eq!(
            validated["resultDigest"],
            read(&repo, "job-results", invocation)["resultDigest"]
        );
    }

    // Optional local qualification artifacts preserve these actual dispatched
    // grouped calls; ordinary tests keep their existing temporary-only output.
    if let Some(directory) = std::env::var_os("CODECLEW_TEST_MODEL_INPUT_ARTIFACTS") {
        let directory = std::path::PathBuf::from(directory);
        fs::create_dir_all(&directory).unwrap();
        for (name, job, prepared, attempt) in [
            ("author-initial", &first_job, &first, &attempts[0]),
            ("author-expanded", &second_job, &second, &attempts[1]),
            (
                "reviewer-initial",
                &review_initial_job,
                &initial_review,
                &review_attempts[0],
            ),
            (
                "reviewer-expanded",
                &review_final_job,
                &final_review,
                &review_attempts[1],
            ),
        ] {
            let input = model_ids::forward_model_input(job, prepared, |canonical_job| {
                let mut packet = canonical_job["payload"]["packet"].clone();
                let expected = packet
                    .as_object_mut()
                    .unwrap()
                    .remove("packetDigest")
                    .unwrap();
                assert_eq!(expected, crate::canonical::hash(&packet).unwrap());
                Ok(())
            })
            .unwrap();
            assert_eq!(input.payload, prepared.model_payload);
            assert_eq!(input.output_schema, prepared.output_schema);
            let artifact = json!({
                "carrier": {"schema": version, "canonicalJob": job, "preparedModel": prepared},
                "modelInput": {"payload": input.payload, "outputSchema": input.output_schema},
                "dispatchObserved": true,
                "protectedFieldsExact": true,
                "canonicalSavedResultsValidated": true,
                "canonicalInputRecord": read(&repo, "job-inputs", attempt["invocation"].as_str().unwrap()),
                "modelInputRecord": read(&repo, "job-model-inputs", attempt["invocation"].as_str().unwrap()),
                "rawResultRecord": read(&repo, "job-model-wire-results", attempt["invocation"].as_str().unwrap()),
                "canonicalResultRecord": read(&repo, "job-results", attempt["invocation"].as_str().unwrap()),
                "roleMapHead": store::read::<Value>(&repo.path(&format!(".codeclew/model-id-maps/{}/{}/head.json", prepared.scope.run, prepared.scope.role)).unwrap(), store::MAX_RECORD).unwrap()
            });
            let name = if version == model_ids::COMPACT_VERSION {
                format!("compact-{name}")
            } else {
                name.to_owned()
            };
            fs::write(
                directory.join(format!("{name}.json")),
                serde_json::to_vec(&artifact).unwrap(),
            )
            .unwrap();
        }
    }
}

#[test]
fn model_ids_host_invalid_delivered_alias_is_retained_and_resume_does_not_dispatch() {
    for version in [model_ids::VERSION, model_ids::COMPACT_VERSION] {
        invalid_delivered_alias(version);
    }
}

fn invalid_delivered_alias(version: &str) {
    for mode in ["unknown", "wrong-scope"] {
        let (_temp, repo, work, path, _) = agent_jobs::model_ids_grouped_author_fixture_setup();
        let counter = DispatchCounter::new();
        let config = opt_in_with_version(&path, "author", &counter, mode, version);
        let outcome =
            agent_jobs::run_operation_draft(&repo, &work.id, Some(&path), false, None, None);
        assert!(
            !outcome
                .as_ref()
                .is_ok_and(|value| value["status"] == "DRAFT")
        );
        let pointer: Value = store::read(
            &repo
                .path(&format!(".codeclew/work/{}/latest-run.json", work.id))
                .unwrap(),
            store::MAX_RECORD,
        )
        .unwrap();
        let report: Value = store::read(
            &repo
                .path(&format!(
                    ".codeclew/jobs/{}.json",
                    pointer["run"].as_str().unwrap()
                ))
                .unwrap(),
            4 * 1024 * 1024,
        )
        .unwrap();
        assert!(report.to_string().contains("MODEL_IDS_INVALID"), "{report}");
        let attempts = report["attempts"].as_array().unwrap();
        assert_eq!(attempts.len(), 2);
        let invocation = attempts[1]["invocation"].as_str().unwrap();
        let raw = read(&repo, "job-model-wire-results", invocation);
        let (_, prepared) = prepared(&repo, &attempts[1]);
        assert!(model_ids::decode_result(&raw["output"]["result"], &prepared).is_err());
        assert!(
            !repo
                .path(&format!(".codeclew/job-results/{invocation}.json"))
                .unwrap()
                .exists()
        );
        assert_eq!(counter.count(), 2);
        let budget: agent_jobs::Budget = serde_json::from_value(config["budget"].clone()).unwrap();
        let reservations = serde_json::to_value(agent_jobs::account(&repo, &budget).unwrap())
            .unwrap()["reservations"]
            .clone();
        let _ = agent_jobs::run_operation_draft(&repo, &work.id, Some(&path), false, None, None);
        assert_eq!(counter.count(), 2, "{mode} resumed with a new dispatch");
        assert_eq!(
            serde_json::to_value(agent_jobs::account(&repo, &budget).unwrap()).unwrap()["reservations"],
            reservations
        );
        assert_eq!(read(&repo, "job-model-wire-results", invocation), raw);
    }
}

#[test]
fn model_ids_host_recovers_prepared_head_and_raw_result_crash_windows() {
    for version in [model_ids::VERSION, model_ids::COMPACT_VERSION] {
        recover_prepared_head_and_raw_result(version);
    }
}

fn recover_prepared_head_and_raw_result(version: &str) {
    let (_temp, repo, work, path, _) = agent_jobs::model_ids_grouped_author_fixture_setup();
    let counter = DispatchCounter::new();
    opt_in_with_version(&path, "author", &counter, "valid", version);
    agent_jobs::interrupt_model_head_publication_once_for_test();
    let interrupted =
        agent_jobs::run_operation_draft(&repo, &work.id, Some(&path), false, None, None)
            .unwrap_err();
    assert!(
        interrupted
            .message
            .contains("RECOVERY_MODEL_HEAD_PUBLICATION_INTERRUPTED"),
        "{interrupted}"
    );
    assert_eq!(counter.count(), 0);
    let pointer: Value = store::read(
        &repo
            .path(&format!(".codeclew/work/{}/latest-run.json", work.id))
            .unwrap(),
        store::MAX_RECORD,
    )
    .unwrap();
    let run = pointer["run"].as_str().unwrap();
    let report: Value = store::read(
        &repo.path(&format!(".codeclew/jobs/{run}.json")).unwrap(),
        4 * 1024 * 1024,
    )
    .unwrap();
    assert_eq!(report["attempts"].as_array().unwrap().len(), 1);
    assert_eq!(report["attempts"][0]["status"], "PREPARED");
    let first_attempt = report["attempts"][0].clone();
    let invocation = first_attempt["invocation"].as_str().unwrap();
    let retained = read(&repo, "job-model-inputs", invocation);
    let (_, first) = prepared(&repo, &first_attempt);
    let head_path = repo
        .path(&format!(".codeclew/model-id-maps/{run}/author/head.json"))
        .unwrap();
    assert!(!head_path.exists());

    // Resuming the same PREPARED call repairs its head. Simulate a separate
    // crash after receiving expansion output, before decode/result storage.
    agent_jobs::interrupt_raw_model_result_once_for_test();
    let interrupted =
        agent_jobs::run_operation_draft(&repo, &work.id, Some(&path), false, None, None)
            .unwrap_err();
    assert!(
        interrupted
            .message
            .contains("RECOVERY_MODEL_WIRE_RESULT_INTERRUPTED"),
        "{interrupted}"
    );
    assert_eq!(counter.count(), 1);
    let head: Value = store::read(&head_path, store::MAX_RECORD).unwrap();
    assert_eq!(head["invocation"], invocation);
    assert_eq!(head["mapDigest"], first.map_digest);
    assert_eq!(read(&repo, "job-model-inputs", invocation), retained);
    let raw = read(&repo, "job-model-wire-results", invocation);
    assert_eq!(raw["output"]["result"]["action"], "expand");
    assert!(
        !repo
            .path(&format!(".codeclew/job-results/{invocation}.json"))
            .unwrap()
            .exists()
    );

    let resumed =
        agent_jobs::run_operation_draft(&repo, &work.id, Some(&path), false, None, None).unwrap();
    assert_eq!(resumed["status"], "DRAFT", "{resumed}");
    assert_eq!(resumed["run"], run);
    assert_eq!(
        counter.count(),
        2,
        "the retained expansion response must not dispatch again"
    );
    let attempts = resumed["attempts"].as_array().unwrap();
    assert_eq!(attempts.len(), 2);
    assert_eq!(attempts[0]["invocation"], invocation);
    let (_, expanded) = prepared(&repo, &attempts[1]);
    assert_eq!(
        &expanded.map.entries[..first.map.entries.len()],
        first.map.entries.as_slice()
    );
    assert_eq!(read(&repo, "job-model-wire-results", invocation), raw);
    assert_eq!(
        read(&repo, "job-results", invocation)["result"]["action"],
        "expand"
    );
    let _ =
        agent_jobs::run_operation_draft(&repo, &work.id, Some(&path), false, None, None).unwrap();
    assert_eq!(counter.count(), 2);
}

#[test]
fn model_ids_host_retained_adapter_failure_cannot_become_success_after_crash() {
    let (_temp, repo, work, path, _) = agent_jobs::model_ids_grouped_author_fixture_setup();
    let counter = DispatchCounter::new();
    let config = opt_in(&path, "author", &counter, "adapter-failure");
    agent_jobs::interrupt_raw_model_result_once_for_test();
    let interrupted =
        agent_jobs::run_operation_draft(&repo, &work.id, Some(&path), false, None, None)
            .unwrap_err();
    assert!(
        interrupted
            .message
            .contains("RECOVERY_MODEL_WIRE_RESULT_INTERRUPTED"),
        "{interrupted}"
    );
    assert_eq!(counter.count(), 1);
    let pointer: Value = store::read(
        &repo
            .path(&format!(".codeclew/work/{}/latest-run.json", work.id))
            .unwrap(),
        store::MAX_RECORD,
    )
    .unwrap();
    let run = pointer["run"].as_str().unwrap();
    let report: Value = store::read(
        &repo.path(&format!(".codeclew/jobs/{run}.json")).unwrap(),
        4 * 1024 * 1024,
    )
    .unwrap();
    let invocation = report["attempts"][0]["invocation"].as_str().unwrap();
    let raw = read(&repo, "job-model-wire-results", invocation);
    assert_eq!(raw["failure"], "ISOLATED_DRIVER_FAILED");
    assert_eq!(raw["output"]["result"]["action"], "expand");
    assert!(
        !repo
            .path(&format!(".codeclew/job-results/{invocation}.json"))
            .unwrap()
            .exists()
    );
    let resumed =
        agent_jobs::run_operation_draft(&repo, &work.id, Some(&path), false, None, None).unwrap();
    assert_ne!(resumed["status"], "DRAFT");
    assert!(
        resumed.to_string().contains("ISOLATED_DRIVER_FAILED"),
        "{resumed}"
    );
    assert_eq!(resumed["attempts"].as_array().unwrap().len(), 1);
    assert_eq!(counter.count(), 1);
    assert_eq!(read(&repo, "job-model-wire-results", invocation), raw);
    assert!(
        !repo
            .path(&format!(".codeclew/job-results/{invocation}.json"))
            .unwrap()
            .exists()
    );
    let budget: agent_jobs::Budget = serde_json::from_value(config["budget"].clone()).unwrap();
    let reservations = serde_json::to_value(agent_jobs::account(&repo, &budget).unwrap()).unwrap()
        ["reservations"]
        .clone();
    let _ =
        agent_jobs::run_operation_draft(&repo, &work.id, Some(&path), false, None, None).unwrap();
    assert_eq!(counter.count(), 1);
    assert_eq!(
        serde_json::to_value(agent_jobs::account(&repo, &budget).unwrap()).unwrap()["reservations"],
        reservations
    );
}
