use super::support::Fixture;
use clew::documentation::{
    cache::{self, CaptureManifest, NON_CACHEABLE},
    check::{self, Check, SourceInputs},
    model::{Entrypoint, Observation, Service, ServiceEvidence, Source},
    store::Repository,
};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    process::Output,
};

const SERVICES: [(&str, &str); 3] = [
    ("billing", "1111111111111111111111111111111111111111"),
    ("inventory", "2222222222222222222222222222222222222222"),
    ("orders", "3333333333333333333333333333333333333333"),
];
const NON_CACHEABLE_REASON: &str =
    "Maven/external-state capture lacks complete build/settings/dependency authority";

fn service(id: &str) -> Service {
    Service {
        schema: "codeclew-documentation-service/1.0".into(),
        id: id.into(),
        title: id.into(),
        repository_id: id.into(),
        repository: format!("https://example.invalid/{id}"),
        language: "java".into(),
        profile: "java-17plus-maven-read-only".into(),
        compilations: vec![":/main".into()],
        source: None,
        modules: None,
        target_ref: "main".into(),
        source_link_template: None,
        contract_files: vec![],
        annotation_processor_paths: vec![],
    }
}

fn evidence(service: &Service, service_digest: String, revision: &str) -> ServiceEvidence {
    let source_text = format!("synthetic source evidence for {}", service.id);
    let source_id = format!("{}-source", service.id);
    let observation_id = format!("{}-flow", service.id);
    let observation_value = json!({"kind":"CALL", "target":"Fixture.dispatch", "order":0});
    let source = Source {
        id: source_id.clone(),
        service: service.id.clone(),
        revision: revision.into(),
        file: "src/Fixture.java".into(),
        start_line: 1,
        end_line: 1,
        text_digest: clew::canonical::hash_bytes(source_text.as_bytes()),
        evidence_digest: clew::canonical::hash(&json!({"fixture":service.id})).unwrap(),
        text: source_text,
        authority: "SYNTHETIC_RETAINED_FIXTURE".into(),
        occurrence: None,
        url: None,
    };
    let observation = Observation {
        id: observation_id.clone(),
        kind: "FLOW".into(),
        service: service.id.clone(),
        symbol: format!("{}.dispatch()", service.id),
        digest: clew::canonical::hash(&observation_value).unwrap(),
        normalized: observation_value,
        source_ids: vec![source_id.clone()],
    };
    let entrypoint = Entrypoint {
        id: format!("{}-dispatch", service.id),
        service: service.id.clone(),
        symbol: format!("{}.dispatch()", service.id),
        kind: "SCHEDULED".into(),
        trigger: json!({"kind":"scheduled-fixture"}),
        source_ids: vec![source_id.clone()],
        dependency_ids: vec![observation_id.clone()],
        boundaries: vec![],
    };
    ServiceEvidence {
        schema: "codeclew-documentation-service-evidence/1.0".into(),
        service: service.id.clone(),
        revision: revision.into(),
        service_digest,
        extractor: "codeclew-documentation-jvm/1.3".into(),
        runtime_mode: "DEVELOPMENT".into(),
        coverage: "PARTIAL".into(),
        boundaries: vec!["EXTERNAL_BUILD_AUTHORITY_UNAVAILABLE".into()],
        entrypoints: vec![entrypoint],
        observations: BTreeMap::from([(observation_id, observation)]),
        sources: BTreeMap::from([(source_id, source)]),
        contracts: BTreeMap::new(),
    }
}

fn copy_tree(source: &Path, destination: &Path) {
    fs::create_dir_all(destination).unwrap();
    for entry in fs::read_dir(source).unwrap() {
        let entry = entry.unwrap();
        let source_path = entry.path();
        let destination_path = destination.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_tree(&source_path, &destination_path);
        } else {
            assert!(
                entry.file_type().unwrap().is_file(),
                "{}",
                source_path.display()
            );
            fs::copy(source_path, destination_path).unwrap();
        }
    }
}

fn tree_bytes(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    fn collect(root: &Path, path: &Path, rows: &mut BTreeMap<PathBuf, Vec<u8>>) {
        for entry in fs::read_dir(path).unwrap() {
            let entry = entry.unwrap();
            let child = entry.path();
            if entry.file_type().unwrap().is_dir() {
                collect(root, &child, rows);
            } else {
                assert!(entry.file_type().unwrap().is_file(), "{}", child.display());
                rows.insert(
                    child.strip_prefix(root).unwrap().to_path_buf(),
                    fs::read(child).unwrap(),
                );
            }
        }
    }
    let mut rows = BTreeMap::new();
    collect(root, root, &mut rows);
    rows
}

#[derive(Debug, PartialEq, Eq)]
enum TreeEntry {
    Directory,
    File(Vec<u8>),
    Symlink(PathBuf),
}

fn tree_inventory(root: &Path) -> BTreeMap<PathBuf, TreeEntry> {
    fn collect(root: &Path, path: &Path, rows: &mut BTreeMap<PathBuf, TreeEntry>) {
        for entry in fs::read_dir(path).unwrap() {
            let entry = entry.unwrap();
            let child = entry.path();
            let relative = child.strip_prefix(root).unwrap().to_path_buf();
            let kind = entry.file_type().unwrap();
            if kind.is_symlink() {
                rows.insert(relative, TreeEntry::Symlink(fs::read_link(&child).unwrap()));
            } else if kind.is_dir() {
                rows.insert(relative.clone(), TreeEntry::Directory);
                collect(root, &child, rows);
            } else {
                assert!(kind.is_file(), "{}", child.display());
                rows.insert(relative, TreeEntry::File(fs::read(child).unwrap()));
            }
        }
    }
    let mut rows = BTreeMap::new();
    collect(root, root, &mut rows);
    rows
}

fn canonical_destination() -> Fixture {
    let mut fixture = Fixture::new();
    fixture.docs = fixture.docs.canonicalize().unwrap();
    fixture
}

fn sqlite_database(cache_directory: &Path) -> PathBuf {
    let layout: Value =
        serde_json::from_slice(&fs::read(cache_directory.join("object-layout.json")).unwrap())
            .unwrap();
    let database_path = Path::new(layout["database"].as_str().unwrap());
    cache_directory.join(database_path.file_name().unwrap())
}

fn sqlite_object(cache_directory: &Path, digest: &str) -> Vec<u8> {
    let connection = rusqlite::Connection::open(sqlite_database(cache_directory)).unwrap();
    connection
        .query_row(
            "SELECT payload FROM objects WHERE digest = ?1",
            [digest],
            |row| row.get(0),
        )
        .unwrap()
}

fn remove_sqlite_object(cache_directory: &Path, digest: &str) {
    let connection = rusqlite::Connection::open(sqlite_database(cache_directory)).unwrap();
    assert_eq!(
        connection
            .execute("DELETE FROM objects WHERE digest = ?1", [digest])
            .unwrap(),
        1,
        "selected synthetic object exists before removal"
    );
    connection
        .execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")
        .unwrap();
    drop(connection);
}

fn restore_sqlite_object(repository: &Repository, digest: &str, payload: &[u8]) {
    let reference =
        clew::documentation::cache::put(repository, "synthetic-closure", payload).unwrap();
    assert_eq!(reference.digest, digest);
    assert_eq!(reference.size, payload.len() as u64);
}

fn expected_import_failure(export: &Path, source_snapshot: &str, captures: &[String]) {
    let destination = canonical_destination();
    let output = run_cli(
        &destination,
        &recovery_args(
            &destination.docs,
            Some(export),
            Some(source_snapshot),
            captures,
        ),
    );
    assert!(!output.status.success());
    let repo = Repository::open(&destination.docs).unwrap();
    assert!(repo.services().unwrap().is_empty());
    assert!(
        !destination
            .docs
            .join(".codeclew/cache/snapshot-import.json")
            .exists()
    );
}

fn source_export(root: &Path, export: &Path) -> (String, Vec<String>, BTreeMap<String, String>) {
    Repository::init(root, "Historical retained-capture fixture").unwrap();
    let repo = Repository::open(root).unwrap();
    for (id, _) in SERVICES {
        repo.service_add(service(id), Some(&repo.input_digest().unwrap()))
            .unwrap();
    }

    let entity = json!({
        "schema":"codeclew-documentation-entity/1.0",
        "id":"historical-entity",
        "title":"Historical entity declaration",
        "description":"Supplied with the original source snapshot for omission testing.",
        "relations":[{"service":SERVICES[0].0,"kind":"owned","origin":"human",
            "rationale":"Historical fixture declaration","confidence":"declared"}],
        "limitations":[]
    });
    repo.atomic(
        "catalog/entities/historical-entity.json",
        &clew::canonical::bytes(&entity).unwrap(),
    )
    .unwrap();
    let scenario = json!({
        "schema":"codeclew-documentation-process/1.0",
        "id":"historical-process",
        "title":"Historical process declaration",
        "summary":"Supplied with the original source snapshot for omission testing.",
        "root":{"service":SERVICES[0].0},
        "interactions":[],
        "maxDepth":4,
        "maxNodes":64,
        "process":{"scope":"Historical fixture process","participants":[SERVICES[0].0],
            "objects":["entity:historical-entity"],"trigger":"Fixture trigger",
            "outcomes":["Fixture outcome"],"linkedSubviews":[]}
    });
    repo.atomic(
        "scenarios/historical-process.yaml",
        &clew::canonical::bytes(&scenario).unwrap(),
    )
    .unwrap();

    let inputs = repo.inputs().unwrap();
    assert!(inputs.entities.contains_key("historical-entity"));
    assert!(inputs.scenarios.contains_key("historical-process"));
    let input_digest = clew::canonical::hash(&inputs).unwrap();
    let mut evidence_by_service = BTreeMap::new();
    let mut revisions = BTreeMap::new();
    for (id, revision) in SERVICES {
        let evidence = evidence(
            &inputs.services[id],
            clew::canonical::hash(&inputs.services[id]).unwrap(),
            revision,
        );
        revisions.insert(id.to_owned(), revision.to_owned());
        evidence_by_service.insert(id.to_owned(), evidence);
    }
    let mut checked = check::assemble(
        input_digest.clone(),
        evidence_by_service,
        BTreeMap::new(),
        &inputs.interactions,
        &inputs.scenarios,
    )
    .unwrap();
    checked.source_inputs = Some(SourceInputs {
        schema: check::SOURCE_INPUTS_SCHEMA.into(),
        input_digest,
        inputs,
        selected_services: SERVICES.iter().map(|(id, _)| (*id).to_owned()).collect(),
        retained_services: BTreeSet::new(),
    });
    let source_manifest = checked.store_manifest(&repo).unwrap();
    assert_eq!(source_manifest.service_manifests.len(), SERVICES.len());

    let captures: Vec<String> = SERVICES
        .iter()
        .map(|(id, _)| format!("{id}-retained.json"))
        .collect();
    for ((service_id, _), name) in SERVICES.iter().zip(&captures) {
        let source_capture = &source_manifest.service_manifests[*service_id];
        assert_eq!(source_capture.cacheability, "REUSABLE");
        assert_eq!(source_capture.coverage, "PARTIAL");
        let mut retained = source_capture.clone();
        cache::mark_non_cacheable(&mut retained, NON_CACHEABLE_REASON);
        assert_eq!(retained.service, *service_id);
        assert_eq!(retained.revision, revisions[*service_id]);
        repo.atomic(
            &format!(".codeclew/cache/{name}"),
            &clew::canonical::bytes(&retained).unwrap(),
        )
        .unwrap();
    }

    let snapshot_bytes = clew::canonical::bytes(&source_manifest).unwrap();
    let snapshot_ref = cache::put(&repo, check::CHECK_MANIFEST_SCHEMA, &snapshot_bytes).unwrap();
    let source_snapshot = format!("{}/{}", snapshot_ref.digest, snapshot_ref.size);
    drop(repo);

    let cache_path = root.join(".codeclew/cache");
    let database = fs::read_dir(&cache_path)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| path.extension().and_then(|extension| extension.to_str()) == Some("sqlite3"))
        .expect("source fixture SQLite database");
    for suffix in ["-wal", "-shm", "-journal"] {
        let sidecar = PathBuf::from(format!("{}{}", database.display(), suffix));
        assert!(
            !sidecar.exists() || fs::metadata(&sidecar).unwrap().len() == 0,
            "source database sidecar was not checkpointed: {}",
            sidecar.display()
        );
    }

    fs::create_dir_all(export).unwrap();
    copy_tree(&cache_path, &export.join("cache"));
    assert!(!export.join("codeclew-docs.yaml").exists());
    assert!(!export.join("catalog").exists());
    assert_eq!(
        fs::read_dir(export).unwrap().count(),
        1,
        "partial export contains only its cache directory"
    );
    for (id, _) in SERVICES {
        assert!(!export.join(id).exists(), "no checkout for {id}");
    }
    (source_snapshot, captures, revisions)
}

fn recovery_args(
    destination: &Path,
    export: Option<&Path>,
    source_snapshot: Option<&str>,
    captures: &[String],
) -> Vec<String> {
    let mut args = vec![
        "docs".into(),
        "snapshot".into(),
        "recover".into(),
        "--root".into(),
        destination.to_str().unwrap().into(),
    ];
    if let Some(export) = export {
        args.extend(["--from-export".into(), export.to_str().unwrap().into()]);
    }
    if let Some(snapshot) = source_snapshot {
        args.extend(["--source-snapshot".into(), snapshot.into()]);
    }
    for capture in captures {
        args.extend(["--capture".into(), capture.clone()]);
    }
    args
}

fn run_cli(fixture: &Fixture, args: &[String]) -> Output {
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    fixture.run_unrooted_raw(&refs)
}

fn success(fixture: &Fixture, args: &[String]) -> Value {
    let output = run_cli(fixture, args);
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

#[test]
fn docsys_t17_selected_partial_export_recovers_offline_with_retained_authority() {
    let temporary = tempfile::tempdir().unwrap();
    let base = temporary.path().canonicalize().unwrap();
    let source_root = base.join("source-docs");
    let export = base.join("partial-export");
    let (source_snapshot, captures, revisions) = source_export(&source_root, &export);
    let export_before = tree_bytes(&export);
    let source_before = tree_bytes(&source_root);

    let mut destination = Fixture::new();
    destination.docs = destination.docs.canonicalize().unwrap();
    assert!(destination.docs.join("codeclew-docs.yaml").is_file());

    // The two import selectors are one explicit mode and are rejected separately.
    for args in [
        recovery_args(&destination.docs, Some(&export), None, &captures),
        recovery_args(&destination.docs, None, Some(&source_snapshot), &captures),
    ] {
        assert!(!run_cli(&destination, &args).status.success());
    }

    let recovered = success(
        &destination,
        &recovery_args(
            &destination.docs,
            Some(&export),
            Some(&source_snapshot),
            &captures,
        ),
    );
    assert_eq!(recovered["status"], "RECOVERED");
    assert_eq!(recovered["sourceSnapshot"], source_snapshot);
    assert_eq!(
        recovered["sourceAuthority"],
        "RETAINED_SOURCE_NOT_REVERIFIED"
    );
    let snapshot = recovered["recoveredSnapshot"].as_str().unwrap();

    let repo = Repository::open(&destination.docs).unwrap();
    let checked = Check::load_snapshot(&repo, snapshot).unwrap();
    assert_eq!(checked.services.len(), SERVICES.len());
    assert_eq!(
        checked
            .services
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        SERVICES.iter().map(|(id, _)| *id).collect::<Vec<_>>()
    );
    assert_eq!(
        checked.source_authorities(),
        SERVICES
            .iter()
            .map(|(id, _)| ((*id).to_owned(), "RETAINED_SOURCE_NOT_REVERIFIED"))
            .collect()
    );
    let (snapshot_digest, snapshot_size) = snapshot.rsplit_once('/').unwrap();
    let saved_manifest: Value = cache::get_json(
        &repo,
        &cache::ObjectRef::new(
            check::CHECK_MANIFEST_SCHEMA.into(),
            snapshot_digest.into(),
            snapshot_size.parse().unwrap(),
        ),
        check::PORTABLE_CACHE_MAX_BYTES,
    )
    .unwrap()
    .unwrap();
    for (id, _) in SERVICES {
        assert_eq!(checked.services[id].revision, revisions[id]);
        assert_eq!(checked.services[id].coverage, "PARTIAL");
        assert_eq!(
            checked.services[id].boundaries,
            vec!["EXTERNAL_BUILD_AUTHORITY_UNAVAILABLE".to_owned()]
        );
        assert_eq!(
            saved_manifest["serviceManifests"][id]["cacheability"],
            NON_CACHEABLE
        );
        assert_eq!(
            saved_manifest["serviceManifests"][id]["reason"],
            NON_CACHEABLE_REASON
        );
    }
    let inputs = repo.inputs().unwrap();
    assert_eq!(inputs.services.len(), SERVICES.len());
    assert!(inputs.entities.is_empty());
    assert!(inputs.scenarios.is_empty());
    assert!(inputs.interactions.is_empty());
    assert!(checked.scenarios.is_empty());
    assert!(
        !repo
            .path(".codeclew/cache/latest-check.json")
            .unwrap()
            .exists()
    );
    assert!(!repo.path(".codeclew/work").unwrap().exists());
    assert!(!repo.path("docs/generated").unwrap().exists());
    assert!(!repo.path("docs/history.html").unwrap().exists());
    for capture in &captures {
        assert_eq!(
            recovered["captures"][capture.as_str()]["cacheability"],
            NON_CACHEABLE
        );
        assert_eq!(
            recovered["captures"][capture.as_str()]["reason"],
            NON_CACHEABLE_REASON
        );
    }
    drop(repo);

    // Reordered selection is an exact idempotent retry over the same snapshot.
    let mut reversed = captures.clone();
    reversed.reverse();
    let repeated = success(
        &destination,
        &recovery_args(
            &destination.docs,
            Some(&export),
            Some(&source_snapshot),
            &reversed,
        ),
    );
    assert_eq!(repeated["recoveredSnapshot"], snapshot);
    assert_eq!(tree_bytes(&export), export_before);
    assert_eq!(tree_bytes(&source_root), source_before);

    // A missing object in one selected closure fails before any service record is registered.
    let broken_export = base.join("missing-payload-export");
    copy_tree(&export, &broken_export);
    let broken_export = broken_export.canonicalize().unwrap();
    let selected_capture = broken_export.join("cache").join(&captures[0]);
    let selected: CaptureManifest =
        serde_json::from_slice(&fs::read(&selected_capture).unwrap()).unwrap();
    let layout: Value =
        serde_json::from_slice(&fs::read(broken_export.join("cache/object-layout.json")).unwrap())
            .unwrap();
    let database_path = Path::new(layout["database"].as_str().unwrap());
    let database = broken_export
        .join("cache")
        .join(database_path.strip_prefix(".codeclew/cache").unwrap());
    let connection = rusqlite::Connection::open(&database).unwrap();
    assert_eq!(
        connection
            .execute(
                "DELETE FROM objects WHERE digest = ?1",
                [&selected.sources.digest],
            )
            .unwrap(),
        1,
        "selected source payload exists in the synthetic export"
    );
    drop(connection);
    let broken_before = tree_bytes(&broken_export);
    let mut rejected = Fixture::new();
    rejected.docs = rejected.docs.canonicalize().unwrap();
    let failed = run_cli(
        &rejected,
        &recovery_args(
            &rejected.docs,
            Some(&broken_export),
            Some(&source_snapshot),
            &captures,
        ),
    );
    assert!(!failed.status.success());
    let rejected_repo = Repository::open(&rejected.docs).unwrap();
    assert!(rejected_repo.services().unwrap().is_empty());
    assert!(
        !rejected_repo
            .path(".codeclew/cache/latest-check.json")
            .unwrap()
            .exists()
    );
    assert_eq!(tree_bytes(&broken_export), broken_before);
    assert_eq!(tree_bytes(&export), export_before);
    assert_eq!(tree_bytes(&source_root), source_before);
}

#[test]
fn docsys_t17_completed_export_retry_checks_receipt_and_destination_objects() {
    let temporary = tempfile::tempdir().unwrap();
    let base = temporary.path().canonicalize().unwrap();
    let source_root = base.join("source-docs");
    let export = base.join("partial-export");
    let (source_snapshot, captures, _) = source_export(&source_root, &export);
    let destination = canonical_destination();
    let imported = success(
        &destination,
        &recovery_args(
            &destination.docs,
            Some(&export),
            Some(&source_snapshot),
            &captures,
        ),
    );
    let snapshot = imported["recoveredSnapshot"].as_str().unwrap();
    let receipt_path = destination
        .docs
        .join(".codeclew/cache/snapshot-import.json");
    let complete_receipt = fs::read(&receipt_path).unwrap();
    let complete_value: Value = serde_json::from_slice(&complete_receipt).unwrap();
    let identity = complete_value["identity"].as_str().unwrap().to_owned();
    let services_path = destination.docs.join("catalog/services");
    let services_before = tree_bytes(&services_path);
    let captures_before: BTreeMap<_, _> = captures
        .iter()
        .map(|name| {
            (
                name.clone(),
                fs::read(destination.docs.join(".codeclew/cache").join(name)).unwrap(),
            )
        })
        .collect();

    for field in ["sourceSnapshot", "sourceStoreId", "services", "captures"] {
        let mut tampered: Value = serde_json::from_slice(&complete_receipt).unwrap();
        match field {
            "sourceSnapshot" => {
                tampered["sourceSnapshot"] = json!(format!("sha256:{}/1", "a".repeat(64)));
            }
            "sourceStoreId" => tampered["sourceStoreId"] = json!("different-source-store"),
            "services" => {
                let services = tampered["services"].as_object_mut().unwrap();
                let id = services.keys().next().unwrap().clone();
                services.insert(id, json!(format!("sha256:{}", "f".repeat(64))));
            }
            "captures" => {
                tampered["captures"]
                    .as_object_mut()
                    .unwrap()
                    .get_mut(captures[0].as_str())
                    .unwrap()["cacheability"] = json!("REUSABLE");
            }
            _ => unreachable!(),
        }
        assert_eq!(tampered["identity"], identity);
        let tampered_bytes = clew::canonical::bytes(&tampered).unwrap();
        fs::write(&receipt_path, &tampered_bytes).unwrap();
        let failed = run_cli(
            &destination,
            &recovery_args(
                &destination.docs,
                Some(&export),
                Some(&source_snapshot),
                &captures,
            ),
        );
        assert!(!failed.status.success(), "tampered {field} was accepted");
        assert_eq!(fs::read(&receipt_path).unwrap(), tampered_bytes);
        assert_eq!(tree_bytes(&services_path), services_before);
        for name in &captures {
            assert_eq!(
                fs::read(destination.docs.join(".codeclew/cache").join(name)).unwrap(),
                captures_before[name]
            );
        }
        fs::write(&receipt_path, &complete_receipt).unwrap();
    }

    // A completed receipt and root do not make a missing destination payload valid.
    let destination_cache = destination.docs.join(".codeclew/cache");
    let selected: CaptureManifest =
        serde_json::from_slice(&fs::read(destination_cache.join(&captures[0])).unwrap()).unwrap();
    let check_digest = snapshot.rsplit_once('/').unwrap().0;
    let check_manifest: Value =
        serde_json::from_slice(&sqlite_object(&destination_cache, check_digest)).unwrap();
    let closure_objects = [
        ("selected capture payload", selected.sources.digest.as_str()),
        (
            "recovered check sourceInputs root",
            check_manifest["sourceInputs"]["digest"].as_str().unwrap(),
        ),
        (
            "recovered check dependenciesIndex root",
            check_manifest["dependenciesIndex"]["digest"]
                .as_str()
                .unwrap(),
        ),
    ];
    let repository = Repository::open(&destination.docs).unwrap();

    // Redirecting a completed receipt to another valid snapshot with the same
    // input digest must not replace the selected capture authority envelope.
    let mut alternate_manifest = check_manifest.clone();
    assert_eq!(
        alternate_manifest["inputDigest"],
        complete_value["inputDigest"]
    );
    alternate_manifest["serviceManifests"][selected.service.as_str()]["cacheability"] =
        json!("REUSABLE");
    alternate_manifest["serviceManifests"][selected.service.as_str()]["reason"] = Value::Null;
    let alternate_bytes = clew::canonical::bytes(&alternate_manifest).unwrap();
    let alternate_ref =
        cache::put(&repository, check::CHECK_MANIFEST_SCHEMA, &alternate_bytes).unwrap();
    let alternate_snapshot = format!("{}/{}", alternate_ref.digest, alternate_ref.size);
    assert_ne!(alternate_snapshot, snapshot);
    assert_eq!(
        Check::load_snapshot(&repository, &alternate_snapshot)
            .unwrap()
            .input_digest,
        complete_value["inputDigest"].as_str().unwrap()
    );
    let mut redirected: Value = serde_json::from_slice(&complete_receipt).unwrap();
    redirected["recoveredSnapshot"] = json!(alternate_snapshot);
    let redirected_bytes = clew::canonical::bytes(&redirected).unwrap();
    fs::write(&receipt_path, &redirected_bytes).unwrap();
    let failed = run_cli(
        &destination,
        &recovery_args(
            &destination.docs,
            Some(&export),
            Some(&source_snapshot),
            &captures,
        ),
    );
    assert!(!failed.status.success(), "alternate snapshot was accepted");
    assert_eq!(fs::read(&receipt_path).unwrap(), redirected_bytes);
    assert_eq!(tree_bytes(&services_path), services_before);
    for name in &captures {
        assert_eq!(
            fs::read(destination.docs.join(".codeclew/cache").join(name)).unwrap(),
            captures_before[name]
        );
    }
    fs::write(&receipt_path, &complete_receipt).unwrap();

    for (description, digest) in closure_objects {
        let payload = sqlite_object(&destination_cache, digest);
        remove_sqlite_object(&destination_cache, digest);
        let failed = run_cli(
            &destination,
            &recovery_args(
                &destination.docs,
                Some(&export),
                Some(&source_snapshot),
                &captures,
            ),
        );
        assert!(
            !failed.status.success(),
            "completed retry trusted missing {description}"
        );
        assert_eq!(fs::read(&receipt_path).unwrap(), complete_receipt);
        assert_eq!(tree_bytes(&services_path), services_before);
        for name in &captures {
            assert_eq!(
                fs::read(destination.docs.join(".codeclew/cache").join(name)).unwrap(),
                captures_before[name]
            );
        }
        restore_sqlite_object(&repository, digest, &payload);
    }
    assert_eq!(complete_value["recoveredSnapshot"], snapshot);
}

#[test]
fn docsys_t17_export_recovery_refuses_private_state_and_orphan_objects() {
    let temporary = tempfile::tempdir().unwrap();
    let base = temporary.path().canonicalize().unwrap();
    let source_root = base.join("source-docs");
    let export = base.join("partial-export");
    let (source_snapshot, captures, _) = source_export(&source_root, &export);

    for retained_kind in ["private-job-state", "orphan-object"] {
        let destination = canonical_destination();
        let repo = Repository::open(&destination.docs).unwrap();
        let marker = match retained_kind {
            "private-job-state" => {
                let marker = repo.path(".codeclew/jobs/retained-run.json").unwrap();
                fs::create_dir_all(marker.parent().unwrap()).unwrap();
                fs::write(&marker, b"retained execution authority").unwrap();
                Some(marker)
            }
            "orphan-object" => {
                let reference =
                    cache::put(&repo, "fixture/orphan-object", b"retained immutable object")
                        .unwrap();
                assert_eq!(
                    cache::get(&repo, &reference, 1024).unwrap(),
                    Some(b"retained immutable object".to_vec())
                );
                None
            }
            _ => unreachable!(),
        };
        drop(repo);

        let failed = run_cli(
            &destination,
            &recovery_args(
                &destination.docs,
                Some(&export),
                Some(&source_snapshot),
                &captures,
            ),
        );
        assert!(
            !failed.status.success(),
            "fresh import admitted {retained_kind}"
        );
        let repo = Repository::open(&destination.docs).unwrap();
        assert!(repo.services().unwrap().is_empty());
        assert!(
            !repo
                .path(".codeclew/cache/snapshot-import.json")
                .unwrap()
                .exists()
        );
        if let Some(marker) = marker {
            assert_eq!(fs::read(marker).unwrap(), b"retained execution authority");
        }
    }
}

#[test]
fn docsys_t17_export_recovery_reads_only_selected_service_closure() {
    let temporary = tempfile::tempdir().unwrap();
    let base = temporary.path().canonicalize().unwrap();
    let source_root = base.join("source-docs");
    let export = base.join("partial-export");
    let (source_snapshot, captures, revisions) = source_export(&source_root, &export);
    let export_cache = export.join("cache");
    let (snapshot_digest, _) = source_snapshot.rsplit_once('/').unwrap();
    let source_check: Value =
        serde_json::from_slice(&sqlite_object(&export_cache, snapshot_digest)).unwrap();
    let source_inputs_digest = source_check["sourceInputs"]["digest"].as_str().unwrap();
    let source_inputs: Value =
        serde_json::from_slice(&sqlite_object(&export_cache, source_inputs_digest)).unwrap();
    let entity_digest = source_inputs["entities"]["historical-entity"]["digest"]
        .as_str()
        .unwrap()
        .to_owned();
    let selected: CaptureManifest =
        serde_json::from_slice(&fs::read(export_cache.join(&captures[0])).unwrap()).unwrap();
    let unselected: CaptureManifest =
        serde_json::from_slice(&fs::read(export_cache.join(&captures[1])).unwrap()).unwrap();
    assert_ne!(selected.service, unselected.service);
    remove_sqlite_object(&export_cache, &unselected.sources.digest);
    remove_sqlite_object(&export_cache, &entity_digest);

    let destination = canonical_destination();
    let imported = success(
        &destination,
        &recovery_args(
            &destination.docs,
            Some(&export),
            Some(&source_snapshot),
            std::slice::from_ref(&captures[0]),
        ),
    );
    assert_eq!(imported["status"], "RECOVERED");
    assert_eq!(imported["services"].as_object().unwrap().len(), 1);
    assert!(
        imported["services"]
            .get(selected.service.as_str())
            .is_some()
    );
    let repo = Repository::open(&destination.docs).unwrap();
    let recovered =
        Check::load_snapshot(&repo, imported["recoveredSnapshot"].as_str().unwrap()).unwrap();
    assert_eq!(recovered.services.len(), 1);
    assert_eq!(
        recovered.services[&selected.service].revision,
        revisions[&selected.service]
    );
    assert_eq!(
        recovered.source_authorities()[&selected.service],
        "RETAINED_SOURCE_NOT_REVERIFIED"
    );
    let inputs = repo.inputs().unwrap();
    assert_eq!(inputs.services.len(), 1);
    assert!(inputs.services.contains_key(&selected.service));
    assert!(inputs.entities.is_empty());
    assert!(inputs.scenarios.is_empty());
}

#[test]
fn docsys_t17_export_recovery_rejects_unsafe_sqlite_and_capture_inputs() {
    use std::os::unix::fs::symlink;

    let temporary = tempfile::tempdir().unwrap();
    let base = temporary.path().canonicalize().unwrap();
    let source_root = base.join("source-docs");
    let export_root = base.join("partial-export");
    let (source_snapshot, captures, _) = source_export(&source_root, &export_root);

    for kind in ["wal", "nonregular-capture", "symlink-capture"] {
        let export = base.join(format!("{kind}-export"));
        copy_tree(&export_root, &export);
        let cache_directory = export.join("cache");
        let selected_path = cache_directory.join(&captures[0]);
        match kind {
            "wal" => {
                let database = sqlite_database(&cache_directory);
                let sidecar = database.with_file_name(format!(
                    "{}-wal",
                    database.file_name().unwrap().to_string_lossy()
                ));
                fs::write(sidecar, b"pending synthetic WAL").unwrap();
            }
            "nonregular-capture" => {
                fs::remove_file(&selected_path).unwrap();
                fs::create_dir(&selected_path).unwrap();
            }
            "symlink-capture" => {
                fs::remove_file(&selected_path).unwrap();
                symlink(cache_directory.join(&captures[1]), &selected_path).unwrap();
            }
            _ => unreachable!(),
        }
        let input_before = tree_inventory(&export);
        expected_import_failure(
            &export,
            &source_snapshot,
            std::slice::from_ref(&captures[0]),
        );
        assert_eq!(
            tree_inventory(&export),
            input_before,
            "{kind} input was modified"
        );
    }
}
