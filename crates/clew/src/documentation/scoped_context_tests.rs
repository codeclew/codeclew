//! End-to-end retained dependency drills through the public docs dispatcher.
use super::*;
use crate::documentation::{cache, check::SourceInputs};
use std::time::Instant;

fn args(
    root: &std::path::Path,
    snapshot: &str,
    service: &str,
    dependency: &str,
    cursor: Option<String>,
) -> ContextArgs {
    ContextArgs {
        root: root.into(),
        service: Some(service.into()),
        scenario: None,
        entrypoint: None,
        symbols: Vec::new(),
        source_ids: Vec::new(),
        dependency_ids: vec![dependency.into()],
        format: ContextFormat::Raw,
        refresh: false,
        snapshot: Some(snapshot.into()),
        cursor,
        limit: 100,
    }
}

fn compare(repo: &Repository, snapshot: &str, service: &str, dependency: &str) -> Value {
    let mut cursor = None;
    let mut measurements = Vec::new();
    let mut item_count = 0;
    loop {
        let request = args(&repo.root, snapshot, service, dependency, cursor.clone());
        cache::take_read_stats();
        let start = Instant::now();
        let (checked, _) =
            check::Check::retained(repo, Some(snapshot), &BTreeSet::from([service.into()]))
                .unwrap();
        let binding = super::super::bindings::baseline(repo).unwrap();
        let subject = format!("service:{service}");
        let narrative = binding
            .as_ref()
            .and_then(|(_, b)| b.narratives.get(&subject));
        let expected = context_from(
            &checked,
            &request,
            narrative,
            "PINNED_SNAPSHOT_NOT_REVERIFIED",
        )
        .unwrap();
        let baseline_ms = start.elapsed().as_secs_f64() * 1000.0;
        let baseline_reads = cache::take_read_stats();
        let start = Instant::now();
        let mut candidate = run(Command::Context(request)).unwrap();
        let candidate_ms = start.elapsed().as_secs_f64() * 1000.0;
        let candidate_reads = cache::take_read_stats();
        let validation = candidate
            .as_object_mut()
            .unwrap()
            .remove("evidenceValidation")
            .unwrap();
        assert_eq!(validation["exhaustiveIntegrityAudit"], false);
        assert_eq!(validation["mode"], "SELECTED_DEPENDENCY_CLOSURE");
        assert_eq!(
            candidate, expected,
            "sources, citations, membership, boundaries, cursor and authority must stay exact"
        );
        assert!(candidate_reads.objects < baseline_reads.objects);
        assert!(candidate_reads.bytes < baseline_reads.bytes);
        item_count += candidate["items"].as_array().unwrap().len();
        measurements.push(
            json!({"baseline":baseline_reads,"candidate":candidate_reads,
            "baselineMs":baseline_ms,"candidateMs":candidate_ms}),
        );
        cursor = candidate["nextCursor"].as_str().map(str::to_owned);
        if cursor.is_none() {
            break;
        }
    }
    json!({"snapshot":snapshot,"service":service,"dependency":dependency,"items":item_count,"pages":measurements})
}

fn fixture() -> (tempfile::TempDir, Repository, String) {
    let temp = tempfile::tempdir().unwrap();
    Repository::init(temp.path(), "Retained method evidence").unwrap();
    let repo = Repository::open(temp.path()).unwrap();
    for id in ["orders", "inventory"] {
        let service: Service = serde_json::from_value(json!({
            "schema":"codeclew-documentation-service/1.0","id":id,"title":id,
            "repositoryId":id,"repository":format!("https://example.invalid/{id}"),
            "language":"java","profile":"java-17plus-maven-read-only",
            "compilations":[":/main"],"targetRef":"main"
        }))
        .unwrap();
        repo.service_add(service, Some(&repo.input_digest().unwrap()))
            .unwrap();
    }
    let inputs = repo.inputs().unwrap();
    let input_digest = repo.input_digest().unwrap();
    let mut services = BTreeMap::new();
    let mut dependencies = BTreeMap::new();
    for service in ["orders", "inventory"] {
        let text = "int price() { return 3; }\n";
        let source = Source {
            id: format!("{service}:source"),
            service: service.into(),
            revision: "a".repeat(40),
            file: "src/Pricing.java".into(),
            start_line: 1,
            end_line: 1,
            text: text.into(),
            text_digest: cache::content_digest(text.as_bytes()),
            evidence_digest: cache::content_digest(text.as_bytes()),
            authority: "COMPILER_EXACT".into(),
            occurrence: None,
            url: Some(format!(
                "https://example.invalid/{service}/blob/{}/src/Pricing.java#L1",
                "a".repeat(40)
            )),
        };
        let mut observations = BTreeMap::new();
        for (id, kind, scope) in [
            ("method", "SYMBOL", ":/main"),
            ("flow", "FLOW", ":/main"),
            ("other-scope", "FLOW", ":/test"),
        ] {
            let observation = Observation {
                id: format!("{service}:{id}"),
                kind: kind.into(),
                service: service.into(),
                symbol: "method:Pricing#price()I".into(),
                normalized: json!({"scope":scope,"kind":"RETURN","text":text}),
                digest: format!("digest-{id}"),
                source_ids: vec![source.id.clone()],
            };
            observations.insert(observation.id.clone(), observation);
        }
        for index in 0..200 {
            let observation = Observation {
                id: format!("{service}:unrelated-{index}"),
                kind: "SYMBOL".into(),
                service: service.into(),
                symbol: format!("method:Pricing#unrelated{index}()I"),
                normalized: json!({"scope":":/main","body":"unrelated evidence ".repeat(100)}),
                digest: format!("digest-{index}"),
                source_ids: vec![source.id.clone()],
            };
            observations.insert(observation.id.clone(), observation);
        }
        dependencies.extend(observations.clone());
        services.insert(
            service.into(),
            ServiceEvidence {
                schema: "codeclew-documentation-service-evidence/1.0".into(),
                service: service.into(),
                revision: "a".repeat(40),
                service_digest: super::super::digest(&inputs.services[service]).unwrap(),
                extractor: "javac-25".into(),
                runtime_mode: "PROJECT_NATIVE".into(),
                coverage: "PARTIAL".into(),
                boundaries: vec!["Runtime behavior is unproven.".into()],
                entrypoints: Vec::new(),
                observations,
                sources: BTreeMap::from([(source.id.clone(), source)]),
                contracts: BTreeMap::from([("declared".into(), json!({"authority":"DECLARED"}))]),
            },
        );
    }
    let checked = check::Check {
        schema: "codeclew-documentation-check/1.0".into(),
        input_digest: input_digest.clone(),
        context_digest: "synthetic-context-with-two-compilation-scopes".into(),
        services,
        dependencies,
        unresolved: BTreeMap::from([("inventory".into(), json!({"runtime":"UNKNOWN"}))]),
        interactions: BTreeMap::new(),
        scenarios: BTreeMap::new(),
        source_inputs: Some(SourceInputs {
            schema: check::SOURCE_INPUTS_SCHEMA.into(),
            input_digest,
            inputs,
            selected_services: BTreeSet::from(["orders".into(), "inventory".into()]),
            retained_services: BTreeSet::new(),
        }),
        composition: None,
    };
    let snapshot = checked.save_snapshot(&repo).unwrap();
    (temp, repo, snapshot)
}

fn mutate_object(repo: &Repository, reference: &cache::ObjectRef, remove: bool) {
    let layout: Value = serde_json::from_slice(
        &std::fs::read(repo.root.join(".codeclew/cache/object-layout.json")).unwrap(),
    )
    .unwrap();
    let connection =
        rusqlite::Connection::open(repo.root.join(layout["database"].as_str().unwrap())).unwrap();
    if remove {
        connection
            .execute(
                "DELETE FROM objects WHERE digest=?1",
                rusqlite::params![reference.digest],
            )
            .unwrap();
    } else {
        connection
            .execute(
                "UPDATE objects SET payload=?1 WHERE digest=?2",
                rusqlite::params![b"corrupt".as_slice(), reference.digest],
            )
            .unwrap();
    }
}

#[test]
fn scoped_context_public_method_drill_preserves_complete_pages_with_less_io() {
    let (_temp, repo, snapshot) = fixture();
    let measured = compare(&repo, &snapshot, "orders", "orders:method");
    assert_eq!(measured["items"], 4); // coverage, selected method+flow, exact source
    let page = &measured["pages"][0];
    println!("SCOPED_CONTEXT_MEASUREMENT {measured}");
    assert!(
        page["candidate"]["bytes"].as_u64().unwrap() < page["baseline"]["bytes"].as_u64().unwrap()
    );
}

#[test]
fn scoped_context_unread_payload_is_not_claimed_verified_and_required_sources_fail_closed() {
    for remove in [false, true] {
        let (_temp, repo, snapshot) = fixture();
        let manifest = check::Check::load_snapshot_manifest(&repo, &snapshot).unwrap();
        mutate_object(
            &repo,
            &manifest.service_manifests["orders"].contracts,
            remove,
        );
        let result = run(Command::Context(args(
            &repo.root,
            &snapshot,
            "orders",
            "orders:method",
            None,
        )))
        .unwrap();
        assert_eq!(
            result["evidenceValidation"]["exhaustiveIntegrityAudit"],
            false
        );
        assert!(check::Check::load_snapshot(&repo, &snapshot).is_err());
        mutate_object(&repo, &manifest.service_manifests["orders"].sources, remove);
        let error = run(Command::Context(args(
            &repo.root,
            &snapshot,
            "orders",
            "orders:method",
            None,
        )))
        .unwrap_err();
        assert_eq!(error.code, ErrorCode::StateCorrupt);
    }
}

#[test]
fn scoped_context_selectors_and_changed_declarations_keep_existing_errors() {
    let (_temp, repo, snapshot) = fixture();
    for dependency in ["unknown", "inventory:method"] {
        let error = run(Command::Context(args(
            &repo.root, &snapshot, "orders", dependency, None,
        )))
        .unwrap_err();
        assert_eq!(error.code, ErrorCode::InvalidInput);
        assert_eq!(error.message, "unknown service dependency");
    }
    let mut changed = repo.services().unwrap()["orders"].clone();
    changed.target_ref = "changed".into();
    repo.service_add(changed, Some(&repo.input_digest().unwrap()))
        .unwrap();
    let error = run(Command::Context(args(
        &repo.root,
        &snapshot,
        "orders",
        "orders:method",
        None,
    )))
    .unwrap_err();
    assert_eq!(error.code, ErrorCode::StaleRequiresReslice);
}

#[test]
#[ignore = "requires an explicitly selected caller-owned retained snapshot; performs no acquisition"]
fn scoped_context_saved_fixture_public_path_measurement() {
    let root = std::env::var("CODECLEW_SCOPED_CONTEXT_ROOT").unwrap();
    let snapshot = std::env::var("CODECLEW_SCOPED_CONTEXT_SNAPSHOT").unwrap();
    let service = std::env::var("CODECLEW_SCOPED_CONTEXT_SERVICE").unwrap();
    let dependency = std::env::var("CODECLEW_SCOPED_CONTEXT_DEPENDENCY").unwrap();
    let repo = Repository::open(&PathBuf::from(root)).unwrap();
    let measured = compare(&repo, &snapshot, &service, &dependency);
    println!("SCOPED_CONTEXT_MEASUREMENT {measured}");
}

#[test]
fn scoped_context_validation_metadata_is_bounded_before_paging() {
    let (_temp, repo, snapshot) = fixture();
    let checked = check::Check::load_snapshot(&repo, &snapshot).unwrap();
    let request = args(&repo.root, &snapshot, "orders", "orders:method", None);
    let error = context_from_with_validation(
        &checked,
        &request,
        None,
        "PINNED_SNAPSHOT_NOT_REVERIFIED",
        Some(json!({"detail":"x".repeat(9 * 1024)})),
    )
    .unwrap_err();
    assert_eq!(error.code, ErrorCode::InvalidInput);
    assert!(error.message.contains("metadata exceeds stdout budget"));
}

#[test]
fn scoped_context_referenced_source_cannot_silently_disappear() {
    let (_temp, repo, snapshot) = fixture();
    let mut manifest = check::Check::load_snapshot_manifest(&repo, &snapshot).unwrap();
    manifest
        .service_manifests
        .get_mut("orders")
        .unwrap()
        .sources = cache::put_json(
        &repo,
        cache::SOURCES_OBJECT_SCHEMA,
        &BTreeMap::<String, Source>::new(),
    )
    .unwrap();
    let reference = cache::put_json(&repo, check::CHECK_MANIFEST_SCHEMA, &manifest).unwrap();
    let changed = format!("{}/{}", reference.digest, reference.size);
    let error = run(Command::Context(args(
        &repo.root,
        &changed,
        "orders",
        "orders:method",
        None,
    )))
    .unwrap_err();
    assert_eq!(error.code, ErrorCode::StateCorrupt);
    assert!(
        error
            .message
            .contains("selected dependency references a missing source")
    );
}
