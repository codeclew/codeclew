use super::*;
use crate::documentation::model::{Observation, ServiceEvidence, Source, SourceOccurrence};
use serde_json::{Value, json};

fn setup() -> (tempfile::TempDir, Repository) {
    let temporary = tempfile::tempdir().unwrap();
    Repository::init(temporary.path(), "Owned capture storage controls").unwrap();
    let repo = Repository::open(temporary.path()).unwrap();
    (temporary, repo)
}

fn evidence(count: usize, text_bytes: usize) -> ServiceEvidence {
    let mut sources = BTreeMap::new();
    let mut contracts = BTreeMap::new();
    let mut observations = BTreeMap::new();
    for i in 0..count {
        let id = format!("owned:module-{i:04}/main:Item");
        let text = format!("// Owned storage fixture {i}\n{}", "x".repeat(text_bytes));
        let digest = cache::content_digest(text.as_bytes());
        sources.insert(
            id.clone(),
            Source {
                id: id.clone(),
                service: "owned".into(),
                revision: "a".repeat(40),
                file: format!("module-{i:04}/src/Item.java"),
                start_line: 1,
                end_line: 2,
                text: text.clone(),
                text_digest: digest.clone(),
                evidence_digest: digest,
                authority: "SOURCE_SYNTAX".into(),
                text_origin: None,
                library_origin: None,
                occurrence: Some(SourceOccurrence {
                    snapshot: "owned-snapshot".into(),
                    blob: format!("owned-blob-{i}"),
                    start_byte: 0,
                    end_byte: text.len(),
                    generated_origin: None,
                }),
                url: None,
            },
        );
        let contract = json!({"declaration":id,"next":format!("owned:module-{:04}/main:Item",(i+1)%count),"revision":"a".repeat(40),"text":text});
        contracts.insert(id.clone(), contract);
        observations.insert(
            id.clone(),
            Observation {
                id: id.clone(),
                service: "owned".into(),
                kind: "OWNED_STORAGE_REFERENCE".into(),
                symbol: id.clone(),
                source_ids: vec![id],
                normalized: json!({"scope":format!("module-{i:04}/main")}),
                digest: "owned-storage-only".into(),
            },
        );
    }
    ServiceEvidence {
        schema: "codeclew-documentation-service-evidence/1.0".into(),
        service: "owned".into(),
        revision: "a".repeat(40),
        service_digest: "owned-input".into(),
        extractor: crate::documentation::model::SOURCE_EXTRACTOR.into(),
        runtime_mode: "RELEASE".into(),
        coverage: "SYNTAX".into(),
        boundaries: vec!["SYNTHETIC_STORAGE_CONTROL_NOT_COMPILER_PROOF".into()],
        entrypoints: Vec::new(),
        sources,
        observations,
        contracts,
    }
}

fn selected_snapshot_context(repo: &Repository, evidence: &ServiceEvidence) {
    let service: crate::documentation::model::Service = serde_json::from_value(json!({
        "schema":"codeclew-documentation-service/1.0", "id":"owned", "title":"Owned storage control",
        "repositoryId":"owned", "repository":"https://example.invalid/owned", "language":"java",
        "profile":"java-17plus-maven-read-only", "compilations":[":/main"], "targetRef":"HEAD"
    })).unwrap();
    repo.atomic(
        "catalog/services/owned.json",
        &crate::canonical::bytes(&service).unwrap(),
    )
    .unwrap();
    let mut evidence = evidence.clone();
    evidence.service_digest = crate::canonical::hash(&service).unwrap();
    let inputs = repo.inputs().unwrap();
    let input_digest = crate::canonical::hash(&inputs).unwrap();
    let mut checked = check::assemble(
        input_digest.clone(),
        BTreeMap::from([("owned".into(), evidence)]),
        BTreeMap::new(),
        &BTreeMap::new(),
        &BTreeMap::new(),
    )
    .unwrap();
    checked.source_inputs = Some(check::SourceInputs {
        schema: check::SOURCE_INPUTS_SCHEMA.into(),
        input_digest,
        inputs,
        selected_services: std::collections::BTreeSet::from(["owned".into()]),
        retained_services: Default::default(),
    });
    let snapshot = checked.save_snapshot(repo).unwrap();
    let (context, actual_snapshot) = check::Check::retained_dependency_context(
        repo,
        Some(&snapshot),
        "owned",
        &checked.services["owned"]
            .observations
            .keys()
            .cloned()
            .collect(),
    )
    .unwrap();
    assert_eq!(actual_snapshot, snapshot);
    assert_eq!(
        context.services["owned"].sources,
        checked.services["owned"].sources
    );
    assert_eq!(context.dependencies, checked.dependencies);
    assert_eq!(
        check::Check::load_snapshot(repo, &snapshot)
            .unwrap()
            .services["owned"],
        checked.services["owned"]
    );
}

#[test]
fn old_wire_and_indexed_capture_preserve_all_records_and_authority() {
    let (_temporary, repo) = setup();
    for (count, size) in [(2, 100), (7, 1024 * 1024)] {
        let evidence = evidence(count, size);
        let legacy_sources =
            cache::put_json(&repo, cache::SOURCES_OBJECT_SCHEMA, &evidence.sources).unwrap();
        assert_eq!(
            load::<Source>(&repo, &legacy_sources, cache::SOURCES_OBJECT_SCHEMA).unwrap(),
            evidence.sources
        );
        let manifest = cache::store_capture(&repo, &evidence).unwrap();
        let loaded = cache::load_capture(&repo, &manifest).unwrap();
        assert_eq!(loaded, evidence);
        let mut legacy_manifest = manifest.clone();
        legacy_manifest.sources = legacy_sources.clone();
        legacy_manifest.contracts =
            cache::put_json(&repo, cache::CONTRACTS_OBJECT_SCHEMA, &evidence.contracts).unwrap();
        assert_eq!(
            cache::load_capture(&repo, &legacy_manifest).unwrap(),
            evidence
        );
        assert_eq!(
            cache::load_capture_sources(&repo, &manifest).unwrap(),
            evidence.sources
        );
        if size > 100 {
            assert_eq!(manifest.sources.schema, INDEX_SCHEMA);
            assert_eq!(manifest.contracts.schema, INDEX_SCHEMA);
            let raw = cache::get(&repo, &manifest.sources, check::PORTABLE_CACHE_MAX_BYTES)
                .unwrap()
                .unwrap();
            let parts =
                part_references(&manifest.sources, &raw, cache::SOURCES_OBJECT_SCHEMA).unwrap();
            cache::take_read_stats();
            cache::take_read_admissions();
            assert_eq!(
                load::<Source>(&repo, &manifest.sources, cache::SOURCES_OBJECT_SCHEMA).unwrap(),
                evidence.sources
            );
            assert_eq!(cache::take_read_stats().objects, parts.len() + 1);
            assert_eq!(cache::take_read_admissions(), 1);
        } else {
            assert_eq!(manifest.sources, legacy_sources);
        }
        selected_snapshot_context(&repo, &evidence);
    }
}

#[test]
fn incomplete_corrupt_or_mismatched_closure_fails_closed() {
    for fault in [
        "missing", "corrupt", "schema", "count", "range", "overlap", "map-kind", "bound",
    ] {
        let (_temporary, repo) = setup();
        let reference = store(
            &repo,
            cache::CONTRACTS_OBJECT_SCHEMA,
            &evidence(5, 1024 * 1024).contracts,
        )
        .unwrap();
        let mut index: Index = cache::get_json(&repo, &reference, check::PORTABLE_CACHE_MAX_BYTES)
            .unwrap()
            .unwrap();
        match fault {
            "missing" => index.parts[0].object.digest = format!("sha256:{}", "f".repeat(64)),
            "corrupt" => {
                let layout: Value = serde_json::from_slice(
                    &std::fs::read(repo.root.join(".codeclew/cache/object-layout.json")).unwrap(),
                )
                .unwrap();
                let connection = rusqlite::Connection::open(
                    repo.root.join(layout["database"].as_str().unwrap()),
                )
                .unwrap();
                let payload = vec![b' '; index.parts[0].object.size as usize];
                connection
                    .execute(
                        "UPDATE objects SET payload=?1 WHERE digest=?2",
                        rusqlite::params![payload, index.parts[0].object.digest],
                    )
                    .unwrap();
            }
            "schema" => index.parts[0].object.schema = cache::SOURCES_OBJECT_SCHEMA.into(),
            "count" => {
                index.parts[0].records += 1;
                index.records += 1;
            }
            "range" => index.parts[0].last.push('z'),
            "overlap" => index.parts[1].first = index.parts[0].first.clone(),
            "map-kind" => index.map_schema = cache::SOURCES_OBJECT_SCHEMA.into(),
            "bound" => index.parts[0].object.size = check::PORTABLE_CACHE_MAX_BYTES + 1,
            _ => unreachable!(),
        }
        let forged = cache::put_json(&repo, INDEX_SCHEMA, &index).unwrap();
        let error = load::<Value>(&repo, &forged, cache::CONTRACTS_OBJECT_SCHEMA).unwrap_err();
        assert!(
            matches!(
                error.code,
                ErrorCode::StateCorrupt | ErrorCode::ResourceLimit
            ),
            "{fault}: {error}"
        );
    }
}

#[test]
#[ignore = "Real 128 MiB boundary and aggregate qualification; run explicitly with one test thread"]
fn real_portable_limit_and_large_aggregate_qualification() {
    let (_temporary, repo) = setup();
    let max = check::PORTABLE_CACHE_MAX_BYTES as usize;
    for size in [max - 1, max, max + 1] {
        let payload = vec![b'x'; size];
        let result = cache::put(&repo, "owned-boundary/1.0", &payload);
        if size <= max {
            assert_eq!(result.unwrap().size, size as u64);
        } else {
            let error = result.unwrap_err();
            assert_eq!(error.code, ErrorCode::SliceBudgetExceeded);
            assert_eq!(error.details.as_ref().unwrap()["actualBytes"], size);
            eprintln!(
                "OWNED_BOUNDARY_FAILURE {}",
                serde_json::to_string(&error).unwrap()
            );
        }
    }
    // Each source/contract stays below the separate 2 MiB portable row bound.
    let evidence = evidence(129, 1024 * 1024);
    for contracts in [false, true] {
        let (schema, bytes) = if contracts {
            (
                cache::CONTRACTS_OBJECT_SCHEMA,
                crate::canonical::bytes(&evidence.contracts).unwrap(),
            )
        } else {
            (
                cache::SOURCES_OBJECT_SCHEMA,
                crate::canonical::bytes(&evidence.sources).unwrap(),
            )
        };
        assert!(bytes.len() > max);
        let error = cache::put(&repo, schema, &bytes).unwrap_err();
        assert_eq!(error.code, ErrorCode::SliceBudgetExceeded);
        eprintln!(
            "LEGACY_AGGREGATE_FAILURE {}",
            serde_json::to_string(&error).unwrap()
        );
    }
    let before: u64 = cache::owned_objects(&repo, usize::MAX)
        .unwrap()
        .values()
        .sum();
    let manifest = cache::store_capture(&repo, &evidence).unwrap();
    let stored = cache::owned_objects(&repo, usize::MAX).unwrap();
    eprintln!(
        "INDEXED_STORAGE_BYTES {}",
        stored.values().sum::<u64>() - before
    );
    for reference in [&manifest.sources, &manifest.contracts] {
        let raw = cache::get(&repo, reference, check::PORTABLE_CACHE_MAX_BYTES)
            .unwrap()
            .unwrap();
        let parsed: Index = serde_json::from_slice(&raw).unwrap();
        assert!(parsed.parts.iter().all(|p| p.object.size <= TARGET as u64));
        eprintln!(
            "INDEXED_PARTS schema={} parts={} maxBytes={}",
            parsed.map_schema,
            parsed.parts.len(),
            parsed.parts.iter().map(|p| p.object.size).max().unwrap()
        );
    }
    let loaded = cache::load_capture(&repo, &manifest).unwrap();
    assert_eq!(loaded, evidence);
    assert_eq!(
        cache::load_capture_sources(&repo, &manifest).unwrap(),
        evidence.sources
    );
    assert_eq!(manifest.sources.schema, INDEX_SCHEMA);
    assert_eq!(manifest.contracts.schema, INDEX_SCHEMA);
    assert_eq!(cache::store_capture(&repo, &evidence).unwrap(), manifest);
    assert_eq!(cache::owned_objects(&repo, usize::MAX).unwrap(), stored);
    drop(loaded);
    selected_snapshot_context(&repo, &evidence);
    let mut too_large = self::evidence(1, 16);
    too_large.sources.values_mut().next().unwrap().text = "x".repeat(max);
    let error = store(&repo, cache::SOURCES_OBJECT_SCHEMA, &too_large.sources).unwrap_err();
    assert_eq!(
        error.details.as_ref().unwrap()["stage"],
        "SERIALIZE_CAPTURE_MAP_RECORD"
    );
    assert_eq!(error.code, ErrorCode::SliceBudgetExceeded);
    eprintln!(
        "SINGLE_RECORD_FAILURE {}",
        serde_json::to_string(&error).unwrap()
    );
    too_large.sources.clear();
    eprintln!(
        "INDEXED_AGGREGATE_PASS sources={} contracts={} observations={}",
        evidence.sources.len(),
        evidence.contracts.len(),
        evidence.observations.len()
    );
}
