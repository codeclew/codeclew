//! Explicit finite synthetic workload. Run one service count per test process.
use super::*;
use crate::documentation::{
    bindings,
    check::Check,
    model::{Narrative, Observation, ServiceEvidence, Source},
    render,
};
use std::{path::Path, time::Instant};

const SPEC: &str = include_str!("../../../../fixtures/bindings-storage-scale/spec.json");

fn text(label: &str, length: usize) -> String {
    format!("{label}: structured retained evidence; ")
        .chars()
        .cycle()
        .take(length)
        .collect()
}

fn fixture(count: usize, changed: bool, input: &str) -> (Check, BTreeMap<String, Narrative>) {
    let spec: Value = serde_json::from_str(SPEC).unwrap();
    let number = |key: &str| spec[key].as_u64().unwrap() as usize;
    let mut checked = Check {
        schema: "codeclew-documentation-check/1.0".into(),
        input_digest: input.into(),
        context_digest: "synthetic-storage-context".into(),
        dependencies: BTreeMap::new(),
        services: BTreeMap::new(),
        unresolved: BTreeMap::new(),
        interactions: BTreeMap::new(),
        scenarios: BTreeMap::new(),
        source_inputs: None,
        composition: None,
    };
    let mut narratives = BTreeMap::new();
    for i in 0..count {
        let service = format!("service-{i:03}");
        let modified = changed && service == spec["changedService"].as_str().unwrap();
        let mut evidence = ServiceEvidence {
            schema: "codeclew-documentation-service-evidence/1.0".into(),
            service: service.clone(),
            revision: if modified { "b" } else { "a" }.repeat(40),
            service_digest: "synthetic".into(),
            extractor: crate::documentation::model::SOURCE_EXTRACTOR.into(),
            runtime_mode: "STATIC".into(),
            coverage: "SYNTAX".into(),
            boundaries: vec![],
            entrypoints: vec![],
            observations: BTreeMap::new(),
            sources: BTreeMap::new(),
            contracts: BTreeMap::new(),
        };
        for source in 0..number("sourcesPerService") {
            let id = format!("{service}-source-{source:02}");
            let body = text(
                &format!(
                    "{id}-{}",
                    if modified && source == 0 {
                        "new"
                    } else {
                        "old"
                    }
                ),
                number("sourceTextBytes"),
            );
            let record: Source = serde_json::from_value(json!({"id":id,"service":service,"revision":evidence.revision,
                "file":format!("src/Operation{source}.java"),"startLine":1,"endLine":100,"text":body,
                "textDigest":canonical::hash_bytes(body.as_bytes()),"evidenceDigest":"synthetic-storage-only","authority":"EXACT_SNAPSHOT_TEXT"})).unwrap();
            evidence.sources.insert(id, record);
        }
        for observation in 0..number("observationsPerService") {
            let id = format!("{service}-fact-{observation:02}");
            let normalized = json!({"name":id,"payload":text(&format!("{id}-{}", if modified && observation == 0 {"new"} else {"old"}),number("observationPayloadBytes")),"resolution":"SOURCE_SYNTAX"});
            evidence.observations.insert(
                id.clone(),
                Observation {
                    id,
                    kind: "SYNTAX_DECLARATION".into(),
                    service: service.clone(),
                    symbol: format!("{service}.operation{observation}"),
                    digest: digest(&normalized).unwrap(),
                    normalized,
                    source_ids: vec![format!(
                        "{service}-source-{:02}",
                        observation % number("sourcesPerService")
                    )],
                },
            );
        }
        // Each operation creates a title and summary binding: exactly four
        // fragments per service, with the configured text bytes in each.
        let operations = (0..number("fragmentsPerService") / 2).map(|operation| {
            let label = format!("{service}-operation-{operation}-{}",if modified && operation == 0 {"new"} else {"old"});
            json!({"id":format!("operation-{operation}"),"title":text(&label,number("fragmentTextBytes")),
                "summary":{"id":"summary","text":text(&label,number("fragmentTextBytes")),
                    "dependencyIds":(0..number("observationsPerService")).filter(|n|n % 2 == operation).map(|n|format!("{service}-fact-{n:02}")).collect::<Vec<_>>(),
                    "sourceIds":(0..number("sourcesPerService")).filter(|n|n % 2 == operation).map(|n|format!("{service}-source-{n:02}")).collect::<Vec<_>>()},"participants":[],"events":[]})
        }).collect::<Vec<_>>();
        let subject = format!("service:{service}");
        narratives.insert(subject.clone(), serde_json::from_value(json!({"schema":"codeclew-documentation-narrative/1.3","subject":subject,"contextDigest":checked.context_digest,"operations":operations,"gaps":{}})).unwrap());
        checked.dependencies.extend(evidence.observations.clone());
        checked.services.insert(service, evidence);
    }
    for start in (0..count).step_by(number("scenarioGroupSize")) {
        let services = (start..(start + number("scenarioGroupSize")).min(count))
            .map(|i| format!("service-{i:03}"))
            .collect::<Vec<_>>();
        let mut ids = Vec::new();
        for fact in 0..number("sharedFactsPerScenario") {
            let id = format!("shared-{start:03}-{fact}");
            let normalized =
                json!({"services":services,"contract":text(&id,number("observationPayloadBytes"))});
            checked.dependencies.insert(
                id.clone(),
                Observation {
                    id: id.clone(),
                    kind: "SHARED_CONTRACT".into(),
                    service: String::new(),
                    symbol: id.clone(),
                    digest: digest(&normalized).unwrap(),
                    normalized,
                    source_ids: services.iter().map(|s| format!("{s}-source-00")).collect(),
                },
            );
            ids.push(id);
        }
        let subject = format!("scenario:shared-{start:03}");
        narratives.insert(subject.clone(),serde_json::from_value(json!({"schema":"codeclew-documentation-narrative/1.3","subject":subject,"contextDigest":checked.context_digest,
            "operations":[{"id":"shared-contract","title":"Cross-service contract","summary":{"id":"summary","text":text(&subject,number("fragmentTextBytes")),"dependencyIds":ids,"sourceIds":services.iter().map(|s|format!("{s}-source-00")).collect::<Vec<_>>()},"participants":[],"events":[]}],"gaps":{}})).unwrap());
    }
    (checked, narratives)
}

fn inventory(root: &Path) -> BTreeMap<String, u64> {
    fn visit(root: &Path, at: &Path, result: &mut BTreeMap<String, u64>) {
        for entry in fs::read_dir(at).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            if entry.file_name() == ".codeclew" {
                continue;
            }
            if path.is_dir() {
                visit(root, &path, result)
            } else {
                result.insert(
                    path.strip_prefix(root).unwrap().to_str().unwrap().into(),
                    entry.metadata().unwrap().len(),
                );
            }
        }
    }
    let mut result = BTreeMap::new();
    visit(root, root, &mut result);
    result
}

#[test]
#[ignore = "finite scale gate: set CODECLEW_BINDINGS_SCALE_COUNT and CODECLEW_BINDINGS_SCALE_OUTPUT"]
fn fixed_structured_scale() {
    let count: usize = std::env::var("CODECLEW_BINDINGS_SCALE_COUNT")
        .unwrap()
        .parse()
        .unwrap();
    let spec: Value = serde_json::from_str(SPEC).unwrap();
    assert!(
        spec["serviceCounts"]
            .as_array()
            .unwrap()
            .iter()
            .any(|n| n.as_u64() == Some(count as u64))
    );
    let output =
        std::path::PathBuf::from(std::env::var_os("CODECLEW_BINDINGS_SCALE_OUTPUT").unwrap());
    fs::create_dir_all(&output).unwrap();
    let (_temporary, repo) = repository();
    let input = repo.input_digest().unwrap();
    let initial_inventory = inventory(&repo.root);
    let mut previous_inventory = initial_inventory.clone();
    let mut legacy_bundle_bytes = 0u64;
    let mut previous_pool = BTreeMap::new();
    let mut previous_partitions: BTreeMap<(String, String), String> = BTreeMap::new();
    let mut rows = Vec::new();
    for (generation, name) in spec["generationOrder"]
        .as_array()
        .unwrap()
        .iter()
        .enumerate()
    {
        let start = Instant::now();
        let (checked, narratives) = fixture(count, generation == 2, &input);
        let binding = render::make_bindings(&checked, narratives).unwrap();
        assert_eq!(
            binding
                .fragments
                .keys()
                .filter(|k| k.starts_with("service:"))
                .count(),
            count * spec["fragmentsPerService"].as_u64().unwrap() as usize
        );
        let original_value = serde_json::to_value(&binding).unwrap();
        let id = publish(&repo, original_value, generation + 1);
        let raw = fs::read(
            repo.path(&format!("docs/generated/{id}/bindings.json"))
                .unwrap(),
        )
        .unwrap();
        let (logical, stats) = resolve(&repo, &raw).unwrap();
        let encoded: Manifest = parse(&raw).unwrap();
        let partitions: BTreeMap<_, _> = encoded
            .partitions
            .iter()
            .map(|p| {
                (
                    (p.owner.kind.clone(), p.owner.id.clone()),
                    p.index.digest.clone(),
                )
            })
            .collect();
        if generation == 1 {
            for (owner, identity) in &previous_partitions {
                if owner.1 != "outputHashes" {
                    assert_eq!(
                        partitions.get(owner),
                        Some(identity),
                        "unchanged evidence partition was rewritten: {owner:?}"
                    );
                }
            }
        }
        if generation == 2 {
            for (owner, identity) in &previous_partitions {
                if owner.1 != "service-000"
                    && owner.1 != "service:service-000"
                    && owner.1 != "scenario:shared-000"
                    && owner.1 != "outputHashes"
                {
                    assert_eq!(
                        partitions.get(owner),
                        Some(identity),
                        "unrelated partition was rewritten: {owner:?}"
                    );
                }
            }
        }
        let inline = bytes(&logical).unwrap();
        let legacy = bindings::validate_retained_bindings(&repo, &inline).unwrap();
        let current = bindings::baseline(&repo).unwrap().unwrap();
        assert_eq!(bytes(&legacy).unwrap(), bytes(&current.1).unwrap());
        bindings::verify_outputs(&repo, &id, &current.1).unwrap();
        let all = inventory(&repo.root);
        let mut publication: Value = serde_json::from_slice(
            &fs::read(
                repo.path(&format!("docs/generated/{id}/publication.json"))
                    .unwrap(),
            )
            .unwrap(),
        )
        .unwrap();
        publication["files"]["bindings.json"] = json!(canonical::hash_bytes(&inline));
        let prefix = format!("docs/generated/{id}/");
        let common: u64 = all
            .iter()
            .filter(|(p, _)| {
                p.starts_with(&prefix)
                    && !p.ends_with("/bindings.json")
                    && !p.ends_with("/publication.json")
            })
            .map(|(_, n)| n)
            .sum();
        legacy_bundle_bytes +=
            common + inline.len() as u64 + bytes(&publication).unwrap().len() as u64;
        let common_root: u64 = all
            .iter()
            .filter(|(p, _)| !p.starts_with("docs/generated/") && !p.starts_with(OBJECT_ROOT))
            .map(|(_, n)| n)
            .sum();
        let pool: BTreeMap<_, _> = all
            .iter()
            .filter(|(p, _)| p.starts_with(OBJECT_ROOT))
            .map(|(p, n)| (p.clone(), *n))
            .collect();
        for (path, size) in &previous_pool {
            assert_eq!(pool.get(path), Some(size));
        }
        let delta: u64 = all
            .iter()
            .filter(|(p, _)| !previous_inventory.contains_key(*p))
            .map(|(_, n)| n)
            .sum();
        let total: u64 = all.values().sum();
        let unchanged_pool = generation == 1 && pool == previous_pool;
        // rusage is a process high-water mark; independent invocation for each
        // count prevents a larger previous workload from contaminating it.
        let mut usage: libc::rusage = unsafe { std::mem::zeroed() };
        assert_eq!(unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut usage) }, 0);
        let peak = usage.ru_maxrss as u64 * if cfg!(target_os = "macos") { 1 } else { 1024 };
        rows.push(json!({"generation":name,"wallMs":start.elapsed().as_millis(),"peakRss":peak,"peakRssUnit":"bytes, process high-water mark",
            "totalBytes":total,"deltaBytes":delta,"fileCount":all.len(),"objectCount":pool.len(),"manifestBytes":raw.len(),
            "hydratedObjects":stats.objects,"hydratedBytes":stats.bytes,"logicalBytes":stats.logical_bytes,
            "legacyInlineBytes":inline.len(),"legacyTotalBytes":legacy_bundle_bytes+common_root,"entirePoolUnchanged":unchanged_pool,"unchangedEvidencePartitionsReused":generation==1,
            "newPoolBytes":pool.iter().filter(|(p,_)|!previous_pool.contains_key(*p)).map(|(_,n)|*n).sum::<u64>()}));
        previous_inventory = all;
        previous_pool = pool;
        previous_partitions = partitions;
    }
    let saving = rows.last().unwrap()["totalBytes"].as_u64().unwrap()
        < rows.last().unwrap()["legacyTotalBytes"].as_u64().unwrap();
    let report = json!({"schema":"codeclew-bindings-storage-scale-result/1.0","services":count,"fixtureDigest":canonical::hash_bytes(SPEC.as_bytes()),"status":"PASS","generations":rows,
        "scope":"Synthetic storage only; three distinct publication IDs exercise generation reuse, not natural no-op render identity. Legacy reference substitutes exact inline logical bindings and recomputed publication metadata in the same file inventory. Hydration metrics cover one explicit resolver call; wall/RSS include construction, production writing, parity validation and baseline expansion.",
        "initialRootBytes":initial_inventory.values().sum::<u64>()});
    fs::write(
        output.join(format!("scale-{count}.json")),
        serde_json::to_vec_pretty(&report).unwrap(),
    )
    .unwrap();
    assert!(saving, "no net storage reduction: {report}");
}
