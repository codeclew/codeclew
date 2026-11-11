//! Owned typed scale fixtures exercise the real portable serializer and reader.
//! They are synthetic records, not a compiler run or private-project evidence.
use super::*;

fn service() -> Service {
    serde_json::from_value(json!({
        "schema":"codeclew-documentation-service/1.0", "id":"orders",
        "title":"Owned scale fixture", "repositoryId":"orders",
        "repository":"https://example.invalid/orders", "language":"java",
        "profile":"java-17plus-maven-read-only", "compilations":[":/main"],
        "targetRef":"HEAD"
    }))
    .unwrap()
}

fn evidence(service: &Service) -> ServiceEvidence {
    ServiceEvidence {
        schema: "codeclew-documentation-service-evidence/1.0".into(),
        service: service.id.clone(),
        revision: "a".repeat(40),
        service_digest: digest(service).unwrap(),
        extractor: SOURCE_EXTRACTOR.into(),
        runtime_mode: "COMMITTED_SOURCE_NO_BUILD".into(),
        coverage: "SYNTAX".into(),
        boundaries: vec!["OWNED_SYNTHETIC_SCALE_NOT_COMPILER_CAPTURE".into()],
        entrypoints: vec![],
        observations: BTreeMap::new(),
        sources: BTreeMap::new(),
        contracts: BTreeMap::new(),
    }
}

fn serialize(service: Service, evidence: &ServiceEvidence) -> (tempfile::TempDir, Manifest) {
    analysis::verify_evidence(evidence).unwrap();
    let mut parts = BTreeMap::new();
    let mut refs = vec![];
    let header = Header {
        schema: evidence.schema.clone(),
        service: evidence.service.clone(),
        revision: evidence.revision.clone(),
        service_digest: evidence.service_digest.clone(),
        extractor: evidence.extractor.clone(),
        runtime_mode: evidence.runtime_mode.clone(),
        coverage: evidence.coverage.clone(),
        boundaries: evidence.boundaries.clone(),
    };
    for (kind, values) in [
        (
            "report",
            vec![report(&service, "CAPTURED", Some(evidence), None).unwrap()],
        ),
        ("header", vec![json!(header)]),
        (
            "sources",
            evidence.sources.values().map(|v| json!(v)).collect(),
        ),
        (
            "observations",
            evidence.observations.values().map(|v| json!(v)).collect(),
        ),
        (
            "entrypoints",
            evidence.entrypoints.iter().map(|v| json!(v)).collect(),
        ),
        (
            "contracts",
            evidence
                .contracts
                .iter()
                .map(|(id, value)| {
                    json!(Contract {
                        id: id.clone(),
                        value: value.clone()
                    })
                })
                .collect(),
        ),
    ] {
        make_parts(kind, values, &mut parts, &mut refs).unwrap();
    }
    let manifest = Manifest {
        schema: SCHEMA.into(),
        service_digest: digest(&service).unwrap(),
        service,
        revision: Some(evidence.revision.clone()),
        producer_version: env!("CARGO_PKG_VERSION").into(),
        compatibility_digest: compatibility().unwrap(),
        outcome: "CAPTURED".into(),
        parts: refs,
        retained: None,
    };
    let directory = tempfile::tempdir().unwrap();
    fs::create_dir(directory.path().join("parts")).unwrap();
    for (path, data) in parts {
        fs::write(directory.path().join(path), data).unwrap();
    }
    fs::write(
        directory.path().join("manifest.json"),
        bytes(&manifest).unwrap(),
    )
    .unwrap();
    (directory, manifest)
}

fn statistics(manifest: &Manifest, result: &Result<Package, ClewError>) -> Value {
    let total_bytes: u64 = manifest.parts.iter().map(|p| p.bytes).sum();
    let records: usize = manifest.parts.iter().map(|p| p.records).sum();
    let maximum = manifest.parts.iter().map(|p| p.bytes).max().unwrap();
    assert!(total_bytes < MAX_TOTAL);
    assert!(maximum < MAX_PART);
    assert!(manifest.parts.len() < MAX_PARTS);
    json!({"records":records,"bytes":total_bytes,"parts":manifest.parts.len(),
        "maximumPartBytes":maximum,"accepted":result.is_ok(),
        "error":result.as_ref().err().map(|e|e.message.as_str())})
}

#[test]
fn owned_serializer_accepts_original_threshold_and_reported_volume() {
    let mut failures = vec![];
    for total in [131_071, 131_072, 131_073, 231_084] {
        let service = service();
        let mut e = evidence(&service);
        e.contracts = (0..total - 2)
            .map(|i| (format!("owned-{i}"), json!({"type":"string"})))
            .collect();
        let (directory, manifest) = serialize(service, &e);
        let result = load(directory.path());
        println!("{}", statistics(&manifest, &result));
        if result.is_err() {
            failures.push(total);
        }
    }
    assert!(
        failures.is_empty(),
        "native serializer output rejected: {failures:?}"
    );
}

#[test]
fn owned_serializer_accepts_reported_typed_sources_observations_and_entrypoints() {
    let (service, e) = reported_typed_evidence();
    let (directory, manifest) = serialize(service, &e);
    let loaded = load(directory.path());
    println!("{}", statistics(&manifest, &loaded));
    let package = loaded.unwrap();
    let roundtrip = package.evidence().unwrap();
    assert_eq!(roundtrip, e);
    assert_eq!(package.records["sources"].len(), 114_777);
    assert_eq!(package.records["observations"].len(), 116_282);
    assert_eq!(package.records["entrypoints"].len(), 23);
}

fn reported_typed_evidence() -> (Service, ServiceEvidence) {
    let service = service();
    let mut e = evidence(&service);
    let text = "class Owned {}";
    let source: Source = serde_json::from_value(json!({
        "id":"owned", "service":"orders", "revision":e.revision,
        "file":"Owned.java", "startLine":1, "endLine":1, "text":text,
        "textDigest":canonical::hash_bytes(text.as_bytes()),
        "evidenceDigest":canonical::hash_bytes(b"owned synthetic input"),
        "authority":"EXACT_SNAPSHOT_TEXT", "url":null
    }))
    .unwrap();
    for i in 0..114_777 {
        let mut s = source.clone();
        s.id = format!("source-{i}");
        e.sources.insert(s.id.clone(), s);
    }
    for i in 0..116_282 {
        let normalized = json!({"owned":i});
        let o = Observation {
            id: format!("fact-{i}"),
            kind: "OWNED_TEST_FACT".into(),
            service: "orders".into(),
            symbol: format!("owned-{i}"),
            digest: digest(&normalized).unwrap(),
            normalized,
            source_ids: vec![format!("source-{}", i % 114_777)],
        };
        e.observations.insert(o.id.clone(), o);
    }
    for i in 0..23 {
        e.entrypoints.push(Entrypoint {
            id: format!("entry-{i}"),
            service: "orders".into(),
            kind: "SOURCE_DECLARATION".into(),
            symbol: format!("owned-{i}"),
            trigger: json!({}),
            source_ids: vec![format!("source-{i}")],
            dependency_ids: vec![format!("fact-{i}")],
            boundaries: vec!["SYNTHETIC_NOT_RUNTIME_REGISTRATION".into()],
        });
    }
    (service, e)
}

fn retained_fixture(
    root: &Path,
    service: &Service,
    e: ServiceEvidence,
    missing_sibling: bool,
) -> (Repository, String) {
    Repository::init(root, "Owned retained export").unwrap();
    let repo = Repository::open(root).unwrap();
    repo.atomic("catalog/services/orders.json", &bytes(service).unwrap())
        .unwrap();
    let mut sibling = service.clone();
    sibling.id = "sibling".into();
    sibling.repository_id = "sibling".into();
    if missing_sibling {
        repo.atomic("catalog/services/sibling.json", &bytes(&sibling).unwrap())
            .unwrap();
    }
    let inputs = repo.inputs().unwrap();
    let input_digest = digest(&inputs).unwrap();
    let mut checked = super::super::check::assemble(
        input_digest.clone(),
        BTreeMap::from([("orders".into(), e)]),
        BTreeMap::new(),
        &BTreeMap::new(),
        &BTreeMap::new(),
    )
    .unwrap();
    checked.source_inputs = Some(super::super::check::SourceInputs {
        schema: super::super::check::SOURCE_INPUTS_SCHEMA.into(),
        input_digest,
        inputs,
        selected_services: BTreeSet::from(["orders".into()]),
        retained_services: BTreeSet::new(),
    });
    let mut manifest = checked.store_manifest(&repo).unwrap();
    let capture = manifest.service_manifests.get_mut("orders").unwrap();
    super::super::cache::mark_non_cacheable(
        capture,
        "Owned fixture: external build authority is incomplete",
    );
    if missing_sibling {
        let mut missing = capture.clone();
        missing.service = "sibling".into();
        missing.service_digest = digest(&sibling).unwrap();
        missing.sources.digest = canonical::hash_bytes(b"unavailable sibling source object");
        manifest.service_manifests.insert("sibling".into(), missing);
    }
    let reference =
        super::super::cache::put_json(&repo, super::super::check::CHECK_MANIFEST_SCHEMA, &manifest)
            .unwrap();
    (repo, format!("{}/{}", reference.digest, reference.size))
}

#[test]
fn retained_export_is_selected_historical_and_independent_of_current_catalog_or_checkout() {
    let temp = tempfile::tempdir().unwrap();
    let original_service = service();
    let mut e = evidence(&original_service);
    e.contracts.insert("owned".into(), json!({"type":"string"}));
    let (repo, snapshot) = retained_fixture(
        &temp.path().join("docs"),
        &original_service,
        e.clone(),
        true,
    );
    assert!(
        super::super::check::Check::load_snapshot(&repo, &snapshot).is_err(),
        "sibling closure is deliberately unavailable"
    );
    let mut moved = original_service.clone();
    moved.title = "Changed current catalog".into();
    repo.atomic("catalog/services/orders.json", &bytes(&moved).unwrap())
        .unwrap();
    let before = repo.input_digest().unwrap();
    let output = temp.path().join("package");
    let result = export_snapshot(&repo, "orders", &snapshot, &output).unwrap();
    assert_eq!(result["status"], "EXPORTED_RETAINED_SNAPSHOT");
    assert_eq!(
        result["retained"]["sourceAuthority"],
        "RETAINED_SOURCE_NOT_REVERIFIED"
    );
    assert_eq!(result["retained"]["cacheability"], "NON_CACHEABLE");
    assert_eq!(
        result["retained"]["automaticReuse"],
        "DISALLOWED_HISTORICAL_EXPORT"
    );
    assert_eq!(repo.input_digest().unwrap(), before);
    assert!(
        !repo
            .path(".codeclew/cache/latest-check.json")
            .unwrap()
            .exists()
    );
    let package = load(&output).unwrap();
    assert_eq!(package.manifest.schema, RETAINED_SCHEMA);
    assert_eq!(package.manifest.service, original_service);
    assert_eq!(package.evidence().unwrap(), e);
    assert!(export_snapshot(&repo, "sibling", &snapshot, &temp.path().join("sibling")).is_err());
    assert!(export_snapshot(&repo, "unknown", &snapshot, &temp.path().join("unknown")).is_err());
    assert!(
        export_snapshot(
            &repo,
            "orders",
            &format!("sha256:{}/1", "f".repeat(64)),
            &temp.path().join("missing")
        )
        .is_err()
    );
    let mut bad = super::super::check::Check::load_snapshot_manifest(&repo, &snapshot).unwrap();
    bad.service_manifests
        .get_mut("orders")
        .unwrap()
        .service_digest = canonical::hash_bytes(b"wrong service identity");
    let r = super::super::cache::put_json(&repo, super::super::check::CHECK_MANIFEST_SCHEMA, &bad)
        .unwrap();
    assert!(
        export_snapshot(
            &repo,
            "orders",
            &format!("{}/{}", r.digest, r.size),
            &temp.path().join("corrupt")
        )
        .is_err()
    );
    let mut bad = super::super::check::Check::load_snapshot_manifest(&repo, &snapshot).unwrap();
    bad.input_digest = canonical::hash_bytes(b"wrong source input identity");
    let r = super::super::cache::put_json(&repo, super::super::check::CHECK_MANIFEST_SCHEMA, &bad)
        .unwrap();
    assert!(
        export_snapshot(
            &repo,
            "orders",
            &format!("{}/{}", r.digest, r.size),
            &temp.path().join("wrong-inputs")
        )
        .is_err()
    );
}

#[test]
fn retained_package_requires_trusted_digest_and_stays_non_cacheable_after_import() {
    let temp = tempfile::tempdir().unwrap();
    let service = service();
    let e = evidence(&service);
    let (repo, snapshot) = retained_fixture(&temp.path().join("producer"), &service, e, false);
    let output = temp.path().join("package");
    let exported = export_snapshot(&repo, "orders", &snapshot, &output).unwrap();
    Repository::init(&temp.path().join("consumer"), "Owned consumer").unwrap();
    let consumer = Repository::open(&temp.path().join("consumer")).unwrap();
    consumer
        .atomic("catalog/services/orders.json", &bytes(&service).unwrap())
        .unwrap();
    assert!(import(&consumer, &output).is_err());
    let policy = Expectation {
        schema: POLICY_SCHEMA.into(),
        service: service.id.clone(),
        repository_id: service.repository_id.clone(),
        service_digest: digest(&service).unwrap(),
        revision: "a".repeat(40),
        manifest_digest: exported["manifestDigest"].as_str().unwrap().into(),
        sequence: 1,
    };
    consumer
        .atomic(&policy_path("orders").unwrap(), &bytes(&policy).unwrap())
        .unwrap();
    assert_eq!(
        import(&consumer, &output).unwrap()["status"],
        "IMPORTED_RETAINED_SNAPSHOT"
    );
    assert_eq!(
        import(&consumer, &output).unwrap()["status"],
        "RETAINED_ALREADY_IMPORTED"
    );
    let loaded = selected(&consumer, &service).unwrap().unwrap();
    assert!(loaded.boundaries.contains(&HISTORICAL_BOUNDARY.into()));
    let capture = super::super::cache::store_capture(&consumer, &loaded).unwrap();
    assert_eq!(capture.cacheability, super::super::cache::NON_CACHEABLE);
    assert_eq!(
        loaded
            .observations
            .values()
            .filter(|o| o.kind == "EVIDENCE_PACKAGE")
            .count(),
        1
    );
    let checked = super::super::check::run(&consumer).unwrap();
    assert_eq!(
        checked.source_authorities()["orders"],
        "RETAINED_SOURCE_NOT_REVERIFIED"
    );
    let saved = checked.store_manifest(&consumer).unwrap();
    assert_eq!(
        saved.service_manifests["orders"].cacheability,
        super::super::cache::NON_CACHEABLE
    );
    let mut manifest: Value =
        serde_json::from_slice(&fs::read(output.join("manifest.json")).unwrap()).unwrap();
    manifest["retained"]["snapshot"] = json!(format!("sha256:{}/1", "b".repeat(64)));
    fs::write(output.join("manifest.json"), bytes(&manifest).unwrap()).unwrap();
    assert!(
        import(&consumer, &output).is_err(),
        "origin changes must change the trusted manifest digest"
    );
}

fn descriptor(index: usize, bytes: u64, records: usize) -> PartRef {
    let digest = canonical::hash_bytes(index.to_string().as_bytes());
    PartRef {
        kind: "contracts".into(),
        path: format!("parts/{}.json", &digest[7..]),
        digest,
        bytes,
        records,
        encoding: "json".into(),
    }
}

#[test]
fn descriptor_budgets_are_independent_exact_and_safe() {
    // No large allocation: these exercise the same envelope preflight used by
    // load/write, not payload admission. Full typed payloads are tested above.
    let mut rows: Vec<PartRef> = (0..MAX_RECORDS / MAX_PART_RECORDS)
        .map(|i| descriptor(i, 1, MAX_PART_RECORDS))
        .collect();
    rows.last_mut().unwrap().records -= 1;
    validate_parts(&rows).unwrap();
    rows.last_mut().unwrap().records += 1;
    validate_parts(&rows).unwrap();
    rows.push(descriptor(rows.len(), 1, 1));
    let error = validate_parts(&rows).unwrap_err();
    assert_eq!(error.details.as_ref().unwrap()["bound"], "records");
    assert_eq!(error.details.as_ref().unwrap()["declared"], MAX_RECORDS + 1);
    let mut totals: Vec<_> = (0..MAX_TOTAL / MAX_PART)
        .map(|i| descriptor(i as usize, MAX_PART, 1))
        .collect();
    totals.last_mut().unwrap().bytes -= 1;
    validate_parts(&totals).unwrap();
    totals.last_mut().unwrap().bytes += 1;
    validate_parts(&totals).unwrap();
    totals.push(descriptor(totals.len(), 1, 1));
    assert_eq!(
        validate_parts(&totals).unwrap_err().details.unwrap()["bound"],
        "totalBytes"
    );
    let mut part = descriptor(0, MAX_PART + 1, 1);
    assert_eq!(
        validate_parts(&[part.clone()])
            .unwrap_err()
            .details
            .unwrap()["bound"],
        "partBytes"
    );
    part.bytes = 1;
    part.records = MAX_PART_RECORDS + 1;
    assert_eq!(
        validate_parts(&[part]).unwrap_err().details.unwrap()["bound"],
        "partRecords"
    );
    let mut parts: Vec<_> = (0..MAX_PARTS).map(|i| descriptor(i, 1, 1)).collect();
    validate_parts(&parts).unwrap();
    parts.push(descriptor(MAX_PARTS, 1, 1));
    assert_eq!(
        validate_parts(&parts).unwrap_err().details.unwrap()["bound"],
        "parts"
    );
    let mut private = descriptor(0, 1, 1);
    private.kind = "private-kind secret-body /private/settings".into();
    let error = validate_parts(&[private]).unwrap_err();
    assert!(!error.message.contains("private"));
    assert!(error.message.contains("kind at index 0"));
    let duplicate = descriptor(0, 1, 1);
    assert!(
        validate_parts(&[duplicate.clone(), duplicate])
            .unwrap_err()
            .message
            .contains("duplicate")
    );
}

#[test]
fn retained_schema_is_explicit_and_legacy_schema_stays_unchanged() {
    let service = service();
    let e = evidence(&service);
    let (directory, manifest) = serialize(service, &e);
    assert_eq!(manifest.schema, SCHEMA);
    let mut value = serde_json::to_value(&manifest).unwrap();
    assert!(value.get("retained").is_none());
    assert!(load(directory.path()).is_ok());
    value["schema"] = json!(RETAINED_SCHEMA);
    fs::write(
        directory.path().join("manifest.json"),
        bytes(&value).unwrap(),
    )
    .unwrap();
    assert!(
        load(directory.path()).is_err(),
        "1.1 cannot omit retained provenance"
    );
}

#[test]
fn reported_typed_scale_exports_from_retained_snapshot_without_capture() {
    let temp = tempfile::tempdir().unwrap();
    let (service, e) = reported_typed_evidence();
    let expected = digest(&e).unwrap();
    let owned_root = std::env::var_os("CODECLEW_TEST_RETAINED_SCALE_ROOT").map(PathBuf::from);
    let root = owned_root
        .clone()
        .unwrap_or_else(|| temp.path().join("docs"));
    assert!(!root.exists(), "select a new owned scale fixture root");
    let (repo, snapshot) = retained_fixture(&root, &service, e, true);
    let manifest_before =
        super::super::check::Check::load_snapshot_manifest(&repo, &snapshot).unwrap();
    let original_capture = manifest_before.service_manifests["orders"].clone();
    let mut changed = service.clone();
    changed.title = "Changed live catalog; original selection stays pinned".into();
    repo.atomic("catalog/services/orders.json", &bytes(&changed).unwrap())
        .unwrap();
    let package = temp.path().join("retained-package");
    let result = export_snapshot(&repo, "orders", &snapshot, &package).unwrap();
    let loaded = load(&package).unwrap();
    let roundtrip = loaded.evidence().unwrap();
    assert_eq!(digest(&roundtrip).unwrap(), expected);
    assert_eq!(
        (
            roundtrip.sources.len(),
            roundtrip.observations.len(),
            roundtrip.entrypoints.len()
        ),
        (114_777, 116_282, 23)
    );
    assert_eq!(
        result["retained"]["captureDigest"],
        digest(&original_capture).unwrap()
    );
    let after = super::super::check::Check::load_snapshot_manifest(&repo, &snapshot).unwrap();
    assert_eq!(after.service_manifests["orders"], original_capture);
    let handoff = json!({"status":"OWNED_RETAINED_TYPED_SCALE_EXPORT_PASS","snapshot":snapshot,
        "service":"orders","originalService":service,"expectedEvidenceDigest":expected,
        "sources":114777,"observations":116282,"entrypoints":23,
        "captureCacheability":original_capture.cacheability,"packageManifestDigest":loaded.digest,
        "serializedParts":loaded.manifest.parts.len(),"serializedBytes":loaded.manifest.parts.iter().map(|p|p.bytes).sum::<u64>(),
        "treatment":"Owned synthetic typed fixture, missing sibling closure and changed live catalog, no source checkout/producer/model. Original cacheability and bindings retained."});
    println!("{handoff}");
    if owned_root.is_some() {
        fs::write(root.join("OWNED-FIXTURE.json"), bytes(&handoff).unwrap()).unwrap();
    }
}
