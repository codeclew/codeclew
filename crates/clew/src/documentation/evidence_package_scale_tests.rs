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
    for total in [131_071, 131_072, 131_073, 157_267, 231_084] {
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
    assert_eq!(roundtrip, &e);
    assert_eq!(roundtrip.sources.len(), 114_777);
    assert_eq!(roundtrip.observations.len(), 116_282);
    assert_eq!(roundtrip.entrypoints.len(), 23);
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
    assert_eq!(package.evidence().unwrap(), &e);
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

fn rewrite_part(
    directory: &Path,
    manifest: &mut Manifest,
    kind: &str,
    edit: impl FnOnce(&mut Value),
) {
    let part = manifest.parts.iter_mut().find(|p| p.kind == kind).unwrap();
    let mut value: Value =
        serde_json::from_slice(&fs::read(directory.join(&part.path)).unwrap()).unwrap();
    edit(&mut value);
    let data = bytes(&value).unwrap();
    let digest = canonical::hash_bytes(&data);
    part.path = format!("parts/{}.json", &digest[7..]);
    part.digest = digest;
    part.bytes = data.len() as u64;
    fs::write(directory.join(&part.path), data).unwrap();
    fs::write(directory.join("manifest.json"), bytes(manifest).unwrap()).unwrap();
}

#[test]
fn malformed_package_and_retained_rows_do_not_echo_private_values() {
    const PRIVATE: &str = "private-body-token /private/build-settings class Secret {}";
    let service = service();
    let e = evidence(&service);
    let (directory, mut manifest) = serialize(service.clone(), &e);
    rewrite_part(directory.path(), &mut manifest, "report", |part| {
        part["records"][0]["errorCode"] = json!(PRIVATE);
    });
    let error = load(directory.path()).err().unwrap();
    assert!(!serde_json::to_string(&error).unwrap().contains(PRIVATE));
    assert!(error.message.contains("diagnostic"));
    let (directory, mut manifest) = serialize(service.clone(), &e);
    rewrite_part(directory.path(), &mut manifest, "header", |part| {
        part[PRIVATE] = json!(PRIVATE);
    });
    let error = load(directory.path()).err().unwrap();
    assert!(!serde_json::to_string(&error).unwrap().contains(PRIVATE));
    assert!(error.message.contains("part JSON"));
    let (directory, manifest) = serialize(service.clone(), &e);
    let mut value = serde_json::to_value(manifest).unwrap();
    value[PRIVATE] = json!(PRIVATE);
    fs::write(
        directory.path().join("manifest.json"),
        bytes(&value).unwrap(),
    )
    .unwrap();
    let error = load(directory.path()).err().unwrap();
    assert!(!serde_json::to_string(&error).unwrap().contains(PRIVATE));
    assert!(error.message.contains("manifest JSON"));

    let temp = tempfile::tempdir().unwrap();
    let (repo, snapshot) = retained_fixture(&temp.path().join("docs"), &service, e, false);
    let mut snapshot =
        super::super::check::Check::load_snapshot_manifest(&repo, &snapshot).unwrap();
    let source = json!({"owned":{"id":"owned","service":"orders","revision":"a".repeat(40),
        "file":"Owned.java","startLine":PRIVATE,"endLine":1,"text":"class Owned {}",
        "textDigest":canonical::hash_bytes(b"class Owned {}"),
        "evidenceDigest":canonical::hash_bytes(b"owned"),"authority":"EXACT_SNAPSHOT_TEXT","url":null}});
    snapshot
        .service_manifests
        .get_mut("orders")
        .unwrap()
        .sources =
        super::super::cache::put_json(&repo, super::super::cache::SOURCES_OBJECT_SCHEMA, &source)
            .unwrap();
    let r =
        super::super::cache::put_json(&repo, super::super::check::CHECK_MANIFEST_SCHEMA, &snapshot)
            .unwrap();
    let error = export_snapshot(
        &repo,
        "orders",
        &format!("{}/{}", r.digest, r.size),
        &temp.path().join("out"),
    )
    .unwrap_err();
    assert!(!serde_json::to_string(&error).unwrap().contains(PRIVATE));
    assert!(error.message.contains("closure is unavailable or corrupt"));
}

#[test]
fn corruption_and_duplicate_rows_still_fail_after_limit_fix() {
    let service = service();
    let mut e = evidence(&service);
    e.contracts.insert("owned".into(), json!({"type":"string"}));
    let (directory, manifest) = serialize(service.clone(), &e);
    let path = directory.path().join(&manifest.parts[0].path);
    fs::write(path, b"private source bytes must not reach the diagnostic").unwrap();
    let error = load(directory.path()).err().unwrap();
    assert!(error.message.contains("digest mismatch"));
    assert!(!error.message.contains("private source"));
    let (directory, mut manifest) = serialize(service.clone(), &e);
    rewrite_part(directory.path(), &mut manifest, "contracts", |part| {
        part["records"]
            .as_array_mut()
            .unwrap()
            .push(json!({"id":"owned","value":1}));
    });
    assert!(
        load(directory.path())
            .err()
            .unwrap()
            .message
            .contains("schema or count mismatch")
    );
    manifest
        .parts
        .iter_mut()
        .find(|p| p.kind == "contracts")
        .unwrap()
        .records = 2;
    fs::write(
        directory.path().join("manifest.json"),
        bytes(&manifest).unwrap(),
    )
    .unwrap();
    assert!(
        load(directory.path())
            .err()
            .unwrap()
            .message
            .contains("duplicate package contract ID")
    );
}

fn composed_fixture() -> (
    tempfile::TempDir,
    Repository,
    String,
    super::super::check::CheckManifest,
    Value,
) {
    let temp = tempfile::tempdir().unwrap();
    let s = service();
    let (repo, original) = retained_fixture(&temp.path().join("docs"), &s, evidence(&s), true);
    let mut parent = super::super::check::Check::load_snapshot_manifest(&repo, &original).unwrap();
    // Match retained-only historical selection, without editing the object store.
    let mut inputs: Value = super::super::cache::get_json(
        &repo,
        &parent.source_inputs,
        super::super::check::PORTABLE_CACHE_MAX_BYTES,
    )
    .unwrap()
    .unwrap();
    inputs["selectedServices"] = json!([]);
    inputs["retainedServices"] = json!(["orders"]);
    parent.source_inputs =
        super::super::cache::put_json(&repo, super::super::source_inputs::MANIFEST_SCHEMA, &inputs)
            .unwrap();
    let p =
        super::super::cache::put_json(&repo, super::super::check::CHECK_MANIFEST_SCHEMA, &parent)
            .unwrap();
    let current_digest = canonical::hash_bytes(b"owned revised declarations");
    let composition = json!({"schema":"codeclew-documentation-composition-manifest/1.0",
        "parent":format!("{}/{}",p.digest,p.size), "composer":canonical::hash_bytes(b"owned composer"),
        "inputDigest":current_digest,
        // This unrelated declaration closure is intentionally unavailable. Export
        // proves selected original capture binding, not whole composition validity.
        "inputs":{"schema":super::super::source_inputs::DECLARATION_MANIFEST_SCHEMA,
            "digest":canonical::hash_bytes(b"unavailable declaration closure"),"size":100},
        "baseline":null,"operations":{},"acceptedVersions":{},"scopes":{}});
    let c = super::super::cache::put_json(
        &repo,
        "codeclew-documentation-composition-manifest/1.0",
        &composition,
    )
    .unwrap();
    let mut derived = parent.clone();
    derived.input_digest = current_digest;
    derived.context_digest = canonical::hash_bytes(b"owned derived context");
    derived.composition = Some(c);
    let r =
        super::super::cache::put_json(&repo, super::super::check::CHECK_MANIFEST_SCHEMA, &derived)
            .unwrap();
    (
        temp,
        repo,
        format!("{}/{}", r.digest, r.size),
        derived,
        composition,
    )
}

#[test]
fn composed_retained_export_binds_unchanged_capture_to_original_inputs() {
    let (temp, repo, snapshot, derived, _) = composed_fixture();
    let output = temp.path().join("package");
    let result = export_snapshot(&repo, "orders", &snapshot, &output).unwrap();
    assert_eq!(result["retained"]["snapshot"], snapshot);
    assert_eq!(result["retained"]["inputDigest"], derived.input_digest);
    let package = load(&output).unwrap();
    assert_eq!(package.evidence().unwrap(), &evidence(&service()));
    assert_eq!(
        package.manifest.retained.unwrap().cacheability,
        "NON_CACHEABLE"
    );
}

#[test]
fn composed_retained_export_refuses_substituted_parent_source_and_capture_bindings() {
    let (temp, repo, _, derived, composition) = composed_fixture();
    for case in ["input", "parent", "source", "capture", "nested"] {
        let mut child = derived.clone();
        let mut envelope = composition.clone();
        match case {
            "input" => {
                envelope["inputDigest"] = json!(canonical::hash_bytes(b"wrong current inputs"))
            }
            "parent" => envelope["parent"] = json!(format!("sha256:{}/1", "f".repeat(64))),
            _ => {
                let mut parent = super::super::check::Check::load_snapshot_manifest(
                    &repo,
                    envelope["parent"].as_str().unwrap(),
                )
                .unwrap();
                match case {
                    "source" => {
                        parent.source_inputs.digest =
                            canonical::hash_bytes(b"wrong source input object")
                    }
                    "capture" => {
                        parent
                            .service_manifests
                            .get_mut("orders")
                            .unwrap()
                            .service_digest = canonical::hash_bytes(b"wrong selected capture")
                    }
                    "nested" => parent.composition = child.composition.clone(),
                    _ => unreachable!(),
                }
                let r = super::super::cache::put_json(
                    &repo,
                    super::super::check::CHECK_MANIFEST_SCHEMA,
                    &parent,
                )
                .unwrap();
                envelope["parent"] = json!(format!("{}/{}", r.digest, r.size));
            }
        }
        child.composition = Some(
            super::super::cache::put_json(
                &repo,
                "codeclew-documentation-composition-manifest/1.0",
                &envelope,
            )
            .unwrap(),
        );
        let r = super::super::cache::put_json(
            &repo,
            super::super::check::CHECK_MANIFEST_SCHEMA,
            &child,
        )
        .unwrap();
        let error = export_snapshot(
            &repo,
            "orders",
            &format!("{}/{}", r.digest, r.size),
            &temp.path().join(case),
        )
        .unwrap_err();
        assert!(
            error.message.contains("composition"),
            "{case}: {}",
            error.message
        );
    }
}

/// One caller-owned byte-scale fixture. No compiler, model, private source or
/// manual database edits. Explicit invocation keeps this outside routine tests.
#[test]
#[ignore = "writes a caller-selected complete large resource fixture"]
fn generate_owned_byte_scale_fixture() {
    let output = std::env::var_os("CODECLEW_TEST_BYTE_SCALE_ROOT").expect("new owned fixture root");
    let root = PathBuf::from(output);
    assert!(root.is_absolute() && !root.exists());
    fs::create_dir_all(&root).unwrap();
    let (service, mut e) = reported_typed_evidence();
    e.coverage = "PARTIAL".into();
    e.boundaries = vec!["OWNED_TYPED_BYTE_SCALE_NOT_A_PRIVATE_COMPILER_CAPTURE".into()];
    for (i, source) in e.sources.values_mut().enumerate() {
        let prefix = format!("class Owned{i} {{ int value() {{ return {i}; }} /*");
        let suffix = "*/ }";
        source.text = format!(
            "{prefix}{}{suffix}",
            "x".repeat(512 - prefix.len() - suffix.len())
        );
        source.file = format!("src/Owned{i}.java");
        source.text_digest = canonical::hash_bytes(source.text.as_bytes());
    }
    for observation in e.observations.values_mut() {
        observation.normalized["payload"] = json!("owned-byte-payload-".repeat(67));
        observation.digest = digest(&observation.normalized).unwrap();
    }
    analysis::verify_evidence(&e).unwrap();
    let oracle_digest = digest(&e).unwrap();
    let source_text_bytes: usize = e.sources.values().map(|s| s.text.len()).sum();
    let (directory, mut manifest) = serialize(service.clone(), &e);
    let total: u64 = manifest.parts.iter().map(|p| p.bytes).sum();
    assert!(
        total > 237_487_403 && total < 512 * 1024 * 1024,
        "actual bytes {total}"
    );
    let (repo, snapshot) = retained_fixture(&root.join("docs"), &service, e, false);
    let saved = super::super::check::Check::load_snapshot_manifest(&repo, &snapshot).unwrap();
    let capture = &saved.service_manifests["orders"];
    manifest.schema = RETAINED_SCHEMA.into();
    manifest.retained = Some(RetainedOrigin {
        schema: "codeclew-documentation-retained-export/1.0".into(),
        snapshot: snapshot.clone(),
        input_digest: saved.input_digest,
        context_digest: saved.context_digest,
        capture_digest: digest(capture).unwrap(),
        cacheability: capture.cacheability.clone(),
        cacheability_reason: capture.reason.clone(),
        source_authority: "RETAINED_SOURCE_NOT_REVERIFIED".into(),
        automatic_reuse: "DISALLOWED_HISTORICAL_EXPORT".into(),
    });
    rewrite_part(directory.path(), &mut manifest, "report", |part| {
        part["records"][0]["stage"] = json!("RETAINED_SNAPSHOT_EXPORT");
    });
    let total: u64 = manifest.parts.iter().map(|p| p.bytes).sum();
    let mut rows = BTreeMap::<String, usize>::new();
    for part in &manifest.parts {
        *rows.entry(part.kind.clone()).or_default() += part.records;
    }
    let fixture = json!({"status":"OWNED_COMPLETE_TYPED_BYTE_SCALE_FROZEN",
        "snapshot":snapshot,"service":"orders","originalService":service,
        "sources":114777,"observations":116282,"entrypoints":23,
        "expectedEvidenceDigest":oracle_digest,"captureCacheability":capture.cacheability,
        "serializedBytes":total,"serializedParts":manifest.parts.len(),"recordsByKind":rows,
        "maxPartBytes":manifest.parts.iter().map(|p|p.bytes).max(),
        "sourceTextBytes":source_text_bytes,"coverage":"PARTIAL",
        "treatment":"Complete owned typed synthetic data through native serializer/cache APIs; no capture/model/private source; oracle frozen before export"});
    fs::write(root.join("OWNED-FIXTURE.json"), bytes(&fixture).unwrap()).unwrap();
    fs::rename(directory.path(), root.join("baseline-package")).unwrap();
    println!("{}", fixture);
}

#[test]
fn guarded_raw_pages_preserve_first_middle_final_and_legacy_shape() {
    let service = service();
    let mut e = evidence(&service);
    e.contracts = (0..1030)
        .map(|i| (format!("owned-{i:04}"), json!({"nullable":null,"value":i})))
        .collect();
    let (directory, manifest) = serialize(service, &e);
    let expected: Vec<_> = e
        .contracts
        .iter()
        .map(|(id, value)| json!({"id":id,"value":value}))
        .collect();
    let binding = digest(&(digest(&manifest).unwrap(), "contracts")).unwrap();
    for start in [0, 511, 1029, 1030] {
        let cursor = (start > 0).then(|| format!("{}:{start}", &binding[7..]));
        let actual = run(Command::Read {
            input: directory.path().into(),
            kind: "contracts".into(),
            cursor: cursor.clone(),
            limit: 2,
        })
        .unwrap();
        let expected = super::super::cli::page(&binding, expected.clone(), cursor.as_deref(), 2,
            json!({"schema":"codeclew-docs-evidence-read/1.0","status":"INTEGRITY_CHECKED_NOT_ADMITTED","manifestDigest":digest(&manifest).unwrap(),"kind":"contracts"})).unwrap();
        assert_eq!(actual, expected);
    }
    let loaded = load(directory.path()).unwrap();
    let late = manifest
        .parts
        .iter()
        .rfind(|p| p.kind == "contracts")
        .unwrap();
    fs::write(
        directory.path().join(&late.path),
        b"changed after validation",
    )
    .unwrap();
    let error = super::super::cli::page_iter(
        &binding,
        1030,
        part_io::RawRows::new(directory.path(), &loaded.manifest, "contracts"),
        Some(&format!("{}:1029", &binding[7..])),
        2,
        json!({}),
    )
    .unwrap_err();
    assert!(error.message.contains("digest mismatch"));
    // In particular, skipping a prefix must not swallow reread errors.
}

#[test]
fn import_late_input_change_keeps_selection_and_leaves_no_partial_namespace() {
    let temp = tempfile::tempdir().unwrap();
    let service = service();
    let mut e = evidence(&service);
    e.contracts.insert("late".into(), json!({"type":"string"}));
    let (input, manifest) = serialize(service.clone(), &e);
    let root = temp.path().join("consumer");
    Repository::init(&root, "Owned consumer").unwrap();
    let repo = Repository::open(&root).unwrap();
    repo.atomic("catalog/services/orders.json", &bytes(&service).unwrap())
        .unwrap();
    let policy = Expectation {
        schema: POLICY_SCHEMA.into(),
        service: service.id.clone(),
        repository_id: service.repository_id.clone(),
        service_digest: digest(&service).unwrap(),
        revision: e.revision.clone(),
        manifest_digest: digest(&manifest).unwrap(),
        sequence: 1,
    };
    repo.atomic(&policy_path("orders").unwrap(), &bytes(&policy).unwrap())
        .unwrap();
    let late = manifest.parts.last().unwrap();
    let path = input.path().join(&late.path);
    let original = fs::read(&path).unwrap();
    let loaded = load(input.path()).unwrap();
    fs::write(&path, b"changed after full typed validation").unwrap();
    assert!(import_loaded(&repo, input.path(), loaded).is_err());
    assert!(
        !repo
            .path(&package_path(&policy.manifest_digest).unwrap())
            .unwrap()
            .exists()
    );
    assert!(
        !repo
            .path(&selection_path("orders").unwrap())
            .unwrap()
            .exists()
    );
    assert_eq!(
        fs::read_dir(root.join("evidence/packages"))
            .unwrap()
            .count(),
        0
    );
    fs::write(&path, &original).unwrap();
    import(&repo, input.path()).unwrap();
    let pointer_path = repo.path(&selection_path("orders").unwrap()).unwrap();
    let before = fs::read(&pointer_path).unwrap();
    let loaded = load(input.path()).unwrap();
    fs::write(&path, b"changed again after full typed validation").unwrap();
    assert!(import_loaded(&repo, input.path(), loaded).is_err());
    assert_eq!(fs::read(&pointer_path).unwrap(), before);
    assert_eq!(
        selected(&repo, &service).unwrap().unwrap().contracts,
        e.contracts
    );
}

#[test]
fn old_byte_envelope_and_late_malformed_or_deep_json_are_checked() {
    for boundary in [
        128 * 1024 * 1024 - 1,
        128 * 1024 * 1024,
        128 * 1024 * 1024 + 1,
    ] {
        let mut parts: Vec<_> = (0..boundary / MAX_PART)
            .map(|i| descriptor(i as usize, MAX_PART, 1))
            .collect();
        if boundary % MAX_PART > 0 {
            parts.push(descriptor(parts.len(), boundary % MAX_PART, 1));
        }
        validate_parts(&parts).unwrap();
    }
    for payload in [b"{not JSON".to_vec(), format!("{{\"schema\":\"{PART_SCHEMA}\",\"kind\":\"contracts\",\"records\":[{{\"id\":\"deep\",\"value\":{}0{}}}]}}", "[".repeat(130),"]".repeat(130)).into_bytes()] {
        let service = service();
        let mut e = evidence(&service);
        e.contracts.insert("last".into(), json!(true));
        let (directory, mut manifest) = serialize(service,&e);
        let late = manifest.parts.last_mut().unwrap();
        late.digest=canonical::hash_bytes(&payload);
        late.path=format!("parts/{}.json", &late.digest[7..]);
        late.bytes=payload.len() as u64;
        fs::write(directory.path().join(&late.path),payload).unwrap();
        fs::write(directory.path().join("manifest.json"),bytes(&manifest).unwrap()).unwrap();
        let error=load(directory.path()).err().unwrap();
        assert!(error.message.contains("invalid evidence part JSON or schema"));
    }
}

#[test]
fn raw_read_keeps_optional_null_fields_omitted_by_typed_reserialization() {
    let service = service();
    let mut e = evidence(&service);
    let text = "class Owned {}";
    let source:Source=serde_json::from_value(json!({"id":"owned","service":"orders","revision":e.revision,
        "file":"Owned.java","startLine":1,"endLine":1,"text":text,
        "textDigest":canonical::hash_bytes(text.as_bytes()),"evidenceDigest":canonical::hash_bytes(b"owned"),
        "authority":"EXACT_SNAPSHOT_TEXT","url":null})).unwrap();
    e.sources.insert(source.id.clone(), source);
    let (directory, mut manifest) = serialize(service, &e);
    rewrite_part(directory.path(), &mut manifest, "sources", |part| {
        part["records"][0]["textOrigin"] = Value::Null;
    });
    let p = load(directory.path()).unwrap();
    assert!(
        serde_json::to_value(&p.evidence().unwrap().sources["owned"])
            .unwrap()
            .get("textOrigin")
            .is_none()
    );
    let page = run(Command::Read {
        input: directory.path().into(),
        kind: "sources".into(),
        cursor: None,
        limit: 1,
    })
    .unwrap();
    assert_eq!(page["items"][0].get("textOrigin"), Some(&Value::Null));
}

/// Owned portable records, not a new compiler capture or transformed cache edit.
fn transformed_fixture() -> (Service, ServiceEvidence) {
    let mut service = service();
    service.profile = "java-17plus-maven-writable-then-seal".into();
    let mut e = evidence(&service);
    e.extractor = EXTRACTOR.into();
    e.runtime_mode = "RELEASE".into();
    e.coverage = "PARTIAL".into();
    for (id, file, text, authority, url) in [
        (
            "transformed",
            "Expanded.java",
            "class Expanded { int value() { return 1; } }",
            crate::generation_service::TRANSFORMED_SOURCE_AUTHORITY,
            None,
        ),
        (
            "original",
            "Original.java",
            "class Original {}",
            "EXACT_SNAPSHOT_TEXT",
            Some(format!(
                "https://example.invalid/orders/blob/{}/Original.java#L1-L1",
                e.revision
            )),
        ),
    ] {
        e.sources.insert(
            id.into(),
            Source {
                id: id.into(),
                service: service.id.clone(),
                revision: e.revision.clone(),
                file: file.into(),
                start_line: 1,
                end_line: 1,
                text: text.into(),
                text_digest: canonical::hash_bytes(text.as_bytes()),
                evidence_digest: canonical::hash_bytes(b"owned admitted compiler inputs"),
                authority: authority.into(),
                text_origin: None,
                library_origin: None,
                occurrence: None,
                url,
            },
        );
    }
    let normalized = json!({"owned":"compiler fixture"});
    e.observations.insert(
        "fact".into(),
        Observation {
            id: "fact".into(),
            kind: "OWNED_TEST_FACT".into(),
            service: service.id.clone(),
            symbol: "Expanded.value".into(),
            digest: digest(&normalized).unwrap(),
            normalized,
            source_ids: vec!["transformed".into(), "original".into()],
        },
    );
    e.entrypoints.push(Entrypoint {
        id: "entry".into(),
        service: service.id.clone(),
        kind: "SOURCE_DECLARATION".into(),
        symbol: "Expanded.value".into(),
        trigger: json!({}),
        source_ids: vec!["transformed".into()],
        dependency_ids: vec!["fact".into()],
        boundaries: vec!["OWNED_NOT_RUNTIME_REGISTRATION".into()],
    });
    (service, e)
}

fn trusted_consumer(root: &Path, service: &Service, manifest_digest: String) -> Repository {
    Repository::init(root, "Owned authority consumer").unwrap();
    let repo = Repository::open(root).unwrap();
    repo.atomic("catalog/services/orders.json", &bytes(service).unwrap())
        .unwrap();
    let policy = Expectation {
        schema: POLICY_SCHEMA.into(),
        service: service.id.clone(),
        repository_id: service.repository_id.clone(),
        service_digest: digest(service).unwrap(),
        revision: "a".repeat(40),
        manifest_digest,
        sequence: 1,
    };
    repo.atomic(&policy_path("orders").unwrap(), &bytes(&policy).unwrap())
        .unwrap();
    repo
}

#[test]
fn transformed_retained_compiler_export_import_preserves_all_records() {
    let temp = tempfile::tempdir().unwrap();
    let (service, e) = transformed_fixture();
    let expected = e.clone();
    let (producer, snapshot) = retained_fixture(&temp.path().join("producer"), &service, e, false);
    let output = temp.path().join("package");
    let exported = export_snapshot(&producer, "orders", &snapshot, &output).unwrap();
    let p = load(&output).unwrap();
    assert_eq!(p.evidence().unwrap(), &expected);
    assert_eq!(p.manifest.schema, RETAINED_SCHEMA);
    let page = run(Command::Read {
        input: output.clone(),
        kind: "sources".into(),
        cursor: None,
        limit: 10,
    })
    .unwrap();
    let transformed = page["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["id"] == "transformed")
        .unwrap();
    assert_eq!(transformed["authority"], "TRANSFORMED_SOURCE");
    assert!(transformed["url"].is_null());
    assert!(transformed.get("textOrigin").is_none());
    let consumer = trusted_consumer(&temp.path().join("consumer"), &service, p.digest.clone());
    assert_eq!(
        import(&consumer, &output).unwrap()["status"],
        "IMPORTED_RETAINED_SNAPSHOT"
    );
    assert_eq!(
        import(&consumer, &output).unwrap()["status"],
        "RETAINED_ALREADY_IMPORTED"
    );
    let stored = load(&consumer.path(&package_path(&p.digest).unwrap()).unwrap()).unwrap();
    assert_eq!(stored.evidence().unwrap(), &expected);
    assert_eq!(stored.digest, exported["manifestDigest"].as_str().unwrap());
    let selected = selected(&consumer, &service).unwrap().unwrap();
    assert_eq!(selected.sources, expected.sources);
    assert_eq!(selected.entrypoints, expected.entrypoints);
    assert!(selected.boundaries.contains(&HISTORICAL_BOUNDARY.into()));
}

#[test]
fn transformed_source_rejects_syntax_without_admitted_compiler_library() {
    let (service, mut e) = transformed_fixture();
    e.extractor = SOURCE_EXTRACTOR.into();
    e.runtime_mode = "COMMITTED_SOURCE_NO_BUILD".into();
    e.coverage = "SYNTAX".into();
    let (package, _) = serialize(service.clone(), &e);
    assert!(
        load(package.path())
            .err()
            .unwrap()
            .message
            .contains("authority or URL")
    );
    let mut source = e.sources["transformed"].clone();
    source.library_origin = Some(LibrarySourceOrigin {
        binding: "not-a-compiler-package".into(),
        original_source_id: "transformed".into(),
    });
    assert!(verify_portable_source(&source, &service, false).is_err());
    let (input, mut manifest) = serialize(service, &e);
    rewrite_part(input.path(), &mut manifest, "sources", |part| {
        let row = part["records"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|r| r["id"] == "transformed")
            .unwrap();
        row["libraryOrigin"] = json!(source.library_origin);
    });
    assert!(
        load(input.path())
            .err()
            .unwrap()
            .message
            .contains("library source has no unique retained binding")
    );
}

fn published_evidence_bytes(repo: &Repository) -> BTreeMap<PathBuf, Vec<u8>> {
    ["evidence/packages", ".codeclew/evidence/selected"]
        .into_iter()
        .flat_map(|prefix| {
            walkdir::WalkDir::new(repo.root.join(prefix))
                .into_iter()
                .map(Result::unwrap)
                .filter(|item| item.file_type().is_file())
                .map(|item| {
                    (
                        item.path().strip_prefix(&repo.root).unwrap().to_path_buf(),
                        fs::read(item.path()).unwrap(),
                    )
                })
                .collect::<Vec<_>>()
        })
        .collect()
}

#[test]
fn transformed_invalid_origins_and_corruption_preserve_prior_import() {
    let temp = tempfile::tempdir().unwrap();
    let (service, e) = transformed_fixture();
    let (valid, manifest) = serialize(service.clone(), &e);
    let consumer = trusted_consumer(
        &temp.path().join("consumer"),
        &service,
        digest(&manifest).unwrap(),
    );
    import(&consumer, valid.path()).unwrap();
    let before = published_evidence_bytes(&consumer);
    for case in [
        "original-url",
        "committed-origin",
        "unsupported-authority",
        "text",
        "evidence-digest",
        "changed-valid-evidence-digest",
        "reference",
    ] {
        let (input, mut manifest) = serialize(service.clone(), &e);
        let kind = if case == "reference" {
            "observations"
        } else {
            "sources"
        };
        rewrite_part(input.path(), &mut manifest, kind, |part| {
            let rows = part["records"].as_array_mut().unwrap();
            let row = if case == "reference" {
                &mut rows[0]
            } else {
                rows.iter_mut().find(|r| r["id"] == "transformed").unwrap()
            };
            match case {
                "original-url" => {
                    row["url"] = json!(format!(
                        "https://example.invalid/orders/blob/{}/Expanded.java#L1-L1",
                        e.revision
                    ))
                }
                "committed-origin" => {
                    let s = &e.sources["transformed"];
                    let mut origin = CommittedTextOrigin {
                        schema: "codeclew-committed-source-text/1.0".into(),
                        revision: s.revision.clone(),
                        file: s.file.clone(),
                        git_blob: "b".repeat(40),
                        content_digest: s.text_digest.clone(),
                        snapshot_digest: canonical::hash_bytes(b"owned snapshot"),
                        start_byte: 0,
                        end_byte: s.text.len(),
                        line_start: 1,
                        line_end: 1,
                        text_digest: s.text_digest.clone(),
                        evidence_digest: s.evidence_digest.clone(),
                        receipt: String::new(),
                    };
                    origin.receipt = digest(&origin).unwrap();
                    row["textOrigin"] = json!(origin);
                }
                "unsupported-authority" => row["authority"] = json!("UNSUPPORTED_SOURCE_AUTHORITY"),
                "text" => row["text"] = json!("changed transformed text"),
                "evidence-digest" => row["evidenceDigest"] = json!("not-an-evidence-digest"),
                "changed-valid-evidence-digest" => {
                    row["evidenceDigest"] = json!(canonical::hash_bytes(
                        b"different untrusted compiler binding"
                    ))
                }
                "reference" => row["sourceIds"] = json!(["missing-source"]),
                _ => unreachable!(),
            }
        });
        // Well-formed opaque compiler hashes are not standalone authenticity
        // proofs; the trusted manifest still prevents admission of changed data.
        if case == "changed-valid-evidence-digest" {
            assert!(load(input.path()).is_ok());
        } else {
            assert!(load(input.path()).is_err(), "{case}");
        }
        assert!(import(&consumer, input.path()).is_err(), "{case}");
        assert_eq!(published_evidence_bytes(&consumer), before, "{case}");
    }
    assert_eq!(
        selected(&consumer, &service).unwrap().unwrap().sources,
        e.sources
    );
}

#[test]
fn transformed_compiler_library_keeps_authority_inside_syntax_consumer() {
    let (_temp, repo, mut binding, mut a, mut b) =
        super::super::library_projection::tests::fixture();
    for source in b.sources.values_mut() {
        source.authority = crate::generation_service::TRANSFORMED_SOURCE_AUTHORITY.into();
        source.url = None;
    }
    binding.capture = super::super::cache::store_capture(&repo, &b).unwrap();
    a.extractor = SOURCE_EXTRACTOR.into();
    a.coverage = "SYNTAX".into();
    a.runtime_mode = "COMMITTED_SOURCE_NO_BUILD".into();
    binding.input.consumer_compilation.clear();
    binding.input.consumer_revision = Some(a.revision.clone());
    for o in a
        .observations
        .values_mut()
        .filter(|o| matches!(o.kind.as_str(), "SYMBOL" | "FLOW"))
    {
        o.normalized.as_object_mut().unwrap().remove("scope");
        o.normalized["authority"] = json!("SYNTAX");
        if o.kind == "FLOW" {
            o.normalized.as_object_mut().unwrap().remove("target");
            o.normalized["libraryCallCandidate"] = json!({"receiver":"Policy","name":"decide","argumentCount":1,"shadowedReceiver":false});
        }
        o.digest = digest(&o.normalized).unwrap();
    }
    let normalized = json!({"syntaxKind":"import","text":"import owned.library.Policy"});
    a.observations.insert(
        "import".into(),
        Observation {
            id: "import".into(),
            kind: "SYNTAX_DETAIL".into(),
            symbol: "import".into(),
            service: a.service.clone(),
            digest: digest(&normalized).unwrap(),
            normalized,
            source_ids: vec!["source-handle".into()],
        },
    );
    let mut service = binding.service.clone();
    service.id = a.service.clone();
    service.repository_id = a.service.clone();
    service.repository = "https://example.invalid/owned/application.git".into();
    service.profile = "source-syntax".into();
    service.compilations.clear();
    service.source = Some(SourceConfig {
        roots: vec!["src".into()],
        dialect: "2.4.10".into(),
        generated_inputs: vec![],
    });
    a.service_digest = digest(&service).unwrap();
    let mut services = BTreeMap::from([(a.service.clone(), a)]);
    super::super::library_projection::apply(
        &repo,
        &BTreeMap::from([(binding.input.id.clone(), binding)]),
        &mut services,
    )
    .unwrap();
    let joined = services.remove("application").unwrap();
    assert_eq!(joined.extractor, SOURCE_EXTRACTOR);
    assert!(
        joined
            .sources
            .values()
            .any(|s| s.authority == "TRANSFORMED_SOURCE" && s.library_origin.is_some())
    );
    let temp = tempfile::tempdir().unwrap();
    let output = temp.path().join("package");
    write_capture_result(service, Ok(joined.clone()), &output, false, None, None).unwrap();
    assert_eq!(load(&output).unwrap().evidence().unwrap(), &joined);
    assert!(
        joined
            .sources
            .values()
            .filter(|s| s.authority == "TRANSFORMED_SOURCE")
            .all(|s| s.url.is_none() && s.text_origin.is_none())
    );
}
