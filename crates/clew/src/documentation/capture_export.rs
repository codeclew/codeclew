//! Recover explicitly selected capture closures from a stopped portable export.
//!
//! The export is opened through an immutable, read-only SQLite connection. Only
//! selected service declarations and capture closures are staged; the rest of
//! the historical source-input graph is intentionally left unread.

use super::{
    bytes, cache,
    check::{self, Check, CheckManifest},
    digest, fact_index, invalid,
    model::{Observation, Service},
    source_inputs,
    sqlite_objects::ImmutableSqliteObjects,
    store::{self, Repository},
};
use crate::error::{ClewError, ErrorCode};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File, OpenOptions},
    io::Read,
    os::unix::fs::OpenOptionsExt,
    path::{Component, Path, PathBuf},
};

const EXPORT_LAYOUT_SCHEMA: &str = "codeclew-documentation-object-layout/2.0";
const EXPORT_LAYOUT_POLICY: &str = "SQLITE_ONLY";
const IMPORT_SCHEMA: &str = "codeclew-documentation-export-recovery/1.0";
const IMPORT_RECORD_SCHEMA: &str = "codeclew-documentation-export-import/1.0";
const STAGE_SCHEMA: &str = "codeclew-documentation-export-import-stage/1.0";
const STAGE_CLEANUP_SCHEMA: &str = "codeclew-documentation-export-import-cleanup/1.0";
const IMPORT_RECORD_PATH: &str = ".codeclew/cache/snapshot-import.json";
const COPY_BATCH_BYTES: usize = 4 * 1024 * 1024;
const COPY_BATCH_OBJECTS: usize = 1024;
const MAX_IMPORT_RECORD_BYTES: u64 = store::MAX_RECORD;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ExportLayout {
    schema: String,
    policy: String,
    store_id: String,
    database: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CaptureReceipt {
    service: String,
    manifest_digest: String,
    cacheability: String,
    reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ImportRecord {
    schema: String,
    identity: String,
    status: String,
    source_snapshot: String,
    source_store_id: String,
    input_digest: String,
    recovered_snapshot: Option<String>,
    captures: BTreeMap<String, CaptureReceipt>,
    services: BTreeMap<String, String>,
    imported_objects: usize,
    imported_bytes: u64,
}

struct ExportReader {
    root: PathBuf,
    store_id: String,
    objects: ImmutableSqliteObjects,
}

struct SelectedCapture {
    name: String,
    raw: Vec<u8>,
    manifest: cache::CaptureManifest,
    receipt: CaptureReceipt,
}

#[derive(Default)]
struct ImportBudget {
    references: BTreeMap<String, u64>,
    bytes: u64,
}

impl ImportBudget {
    fn observe(&mut self, reference: &cache::ObjectRef) -> Result<(), ClewError> {
        match self.references.get(&reference.digest) {
            Some(size) if *size != reference.size => {
                return Err(import_corrupt("one source digest has conflicting sizes"));
            }
            Some(_) => return Ok(()),
            None => {}
        }
        self.bytes = self.bytes.checked_add(reference.size).ok_or_else(|| {
            ClewError::new(
                ErrorCode::ResourceLimit,
                "selected export byte count overflowed",
            )
        })?;
        self.references
            .insert(reference.digest.clone(), reference.size);
        Ok(())
    }

    fn observe_file(&mut self, digest: &str, size: u64) -> Result<(), ClewError> {
        let reference = cache::ObjectRef::new("export-file/1.0".into(), digest.into(), size);
        self.observe(&reference)
    }
}

#[derive(Default)]
struct CopyBatch {
    copied: BTreeSet<String>,
    payloads: Vec<Vec<u8>>,
    bytes: usize,
}

impl CopyBatch {
    fn push(
        &mut self,
        stage: &Repository,
        reference: &cache::ObjectRef,
        payload: Vec<u8>,
    ) -> Result<(), ClewError> {
        if self.copied.contains(&reference.digest) {
            return Ok(());
        }
        if self.payloads.len() >= COPY_BATCH_OBJECTS
            || (!self.payloads.is_empty()
                && self.bytes.saturating_add(payload.len()) > COPY_BATCH_BYTES)
        {
            self.flush(stage)?;
        }
        self.bytes = self.bytes.saturating_add(payload.len());
        self.copied.insert(reference.digest.clone());
        self.payloads.push(payload);
        if self.bytes >= COPY_BATCH_BYTES || self.payloads.len() >= COPY_BATCH_OBJECTS {
            self.flush(stage)?;
        }
        Ok(())
    }

    fn flush(&mut self, stage: &Repository) -> Result<(), ClewError> {
        if self.payloads.is_empty() {
            return Ok(());
        }
        let payloads = self.payloads.iter().map(Vec::as_slice).collect::<Vec<_>>();
        cache::put_batch(stage, "codeclew-documentation-export-object/1.0", &payloads)?;
        self.payloads.clear();
        self.bytes = 0;
        Ok(())
    }
}

pub(super) fn run(
    destination: &Path,
    export: &Path,
    source_snapshot: &str,
    capture_names: &[String],
) -> Result<Value, ClewError> {
    if capture_names.is_empty() || capture_names.len() > store::MAX_RECORDS {
        return Err(invalid(
            "export recovery requires a bounded explicit capture list",
        ));
    }
    let reader = ExportReader::open(export)?;
    let destination_root = checked_directory(destination)?;
    reject_path_overlap(&reader.root, &destination_root)?;
    validate_destination_paths(&destination_root)?;
    let destination_repo = Repository::open(&destination_root)?;
    // Serialize this import against every destination writer. The stage uses
    // its own nested repository lock and does not reacquire this one.
    let _destination_lock = destination_repo.lock()?;

    let mut budget = ImportBudget::default();
    let (_snapshot_ref, snapshot_manifest) =
        read_source_snapshot(&reader, source_snapshot, &mut budget)?;
    let selected = read_selected_captures(&reader, capture_names, &snapshot_manifest, &mut budget)?;
    let selected_services = selected
        .iter()
        .map(|capture| capture.manifest.service.clone())
        .collect::<BTreeSet<_>>();
    let source_services = source_inputs::load_selected_services(
        &snapshot_manifest.source_inputs,
        &snapshot_manifest.input_digest,
        &selected_services,
        |reference| {
            budget.observe(reference)?;
            read_required(&reader, reference)
        },
    )?;
    for capture in &selected {
        let service = source_services
            .get(&capture.manifest.service)
            .ok_or_else(|| invalid("selected source service is missing"))?;
        if digest(service)? != capture.manifest.service_digest {
            return Err(invalid(
                "selected capture does not match its source snapshot service definition",
            ));
        }
    }

    let services = source_services
        .iter()
        .map(|(id, service)| Ok((id.clone(), digest(service)?)))
        .collect::<Result<BTreeMap<_, _>, _>>()?;
    let captures = selected
        .iter()
        .map(|capture| (capture.name.clone(), capture.receipt.clone()))
        .collect::<BTreeMap<_, _>>();
    let identity = digest(&json!({
        "sourceSnapshot": source_snapshot,
        "sourceStoreId": reader.store_id,
        "captures": selected.iter().map(|capture| (&capture.name, &capture.receipt.manifest_digest)).collect::<BTreeMap<_, _>>(),
        "services": services,
        "destinationTitle": destination_repo.manifest.title,
    }))?;
    let mut planned_inputs = destination_repo.inputs()?;
    planned_inputs.services = source_services.clone();
    let planned_input_digest = digest(&planned_inputs)?;
    let current_record = read_import_record(&destination_repo)?;
    let mut record = if let Some(record) = current_record {
        validate_import_record(&record)?;
        if record.identity != identity {
            return Err(ClewError::new(
                ErrorCode::WwConflict,
                "destination already has a different snapshot export import",
            ));
        }
        if record.source_snapshot != source_snapshot
            || record.source_store_id != reader.store_id
            || record.captures != captures
            || record.services != services
            || record.input_digest != planned_input_digest
        {
            return Err(import_corrupt(
                "snapshot import receipt fields do not match the selected export",
            ));
        }
        if record.status == "COMPLETE" {
            verify_completed_import(
                &destination_repo,
                &record,
                &source_services,
                &selected,
                &identity,
            )?;
            cleanup_stage(
                &destination_repo,
                &stage_path(&destination_repo, &identity)?,
                &identity,
            )?;
            return Ok(result(&record));
        }
        record
    } else {
        validate_empty_destination(&destination_repo)?;
        validate_cache_inventory(&destination_repo, &BTreeSet::new(), None, false)?;
        let record = ImportRecord {
            schema: IMPORT_RECORD_SCHEMA.into(),
            identity: identity.clone(),
            status: "IN_PROGRESS".into(),
            source_snapshot: source_snapshot.into(),
            source_store_id: reader.store_id.clone(),
            input_digest: planned_input_digest.clone(),
            recovered_snapshot: None,
            captures: captures.clone(),
            services: services.clone(),
            imported_objects: budget.references.len(),
            imported_bytes: budget.bytes,
        };
        ensure_import_record_capacity(&record)?;
        write_import_record(&destination_repo, &record)?;
        record
    };
    // A completed receipt adds the recovered snapshot handle and final byte
    // counters. Prove that the worst-case record still fits before staging or
    // promoting any data, so a successful import cannot become unreadable.
    ensure_import_record_capacity(&record)?;
    validate_destination_state(
        &destination_repo,
        &source_services,
        &selected,
        &identity,
        true,
    )?;

    let stage_path = stage_path(&destination_repo, &identity)?;
    let stage_repo = open_or_create_stage(&destination_repo, &stage_path, &identity)?;
    ensure_stage_services(&stage_repo, &source_services, &identity)?;
    let mut batch = CopyBatch::default();
    for capture in &selected {
        budget.observe_file(&capture.receipt.manifest_digest, capture.raw.len() as u64)?;
        copy_capture_closure(
            &reader,
            &stage_repo,
            &capture.manifest,
            &mut budget,
            &mut batch,
        )?;
        write_capture(&stage_repo, capture)?;
    }
    batch.flush(&stage_repo)?;
    let mut source_ids = BTreeSet::new();
    let mut observation_ids = BTreeSet::new();
    let mut entrypoint_ids = BTreeSet::new();
    for capture in &selected {
        let evidence = cache::load_capture(&stage_repo, &capture.manifest)?;
        validate_evidence(
            &capture.manifest,
            &evidence,
            &mut source_ids,
            &mut observation_ids,
            &mut entrypoint_ids,
        )?;
    }
    let staged_result = super::capture_recovery::run(
        &stage_repo.root,
        &selected
            .iter()
            .map(|capture| capture.name.clone())
            .collect::<Vec<_>>(),
    )?;
    let recovered_snapshot = staged_result["snapshot"]
        .as_str()
        .ok_or_else(|| invalid("staged capture recovery did not return a snapshot"))?
        .to_owned();
    let staged_manifest = Check::load_snapshot_manifest(&stage_repo, &recovered_snapshot)?;
    if staged_manifest.input_digest != stage_repo.input_digest()?
        || staged_manifest.service_manifests.len() != selected.len()
    {
        return Err(import_corrupt(
            "staged recovery snapshot is not bound to selected services",
        ));
    }

    if record.schema != IMPORT_RECORD_SCHEMA
        || record.status != "IN_PROGRESS"
        || record.captures != captures
        || record.services != services
        || record.input_digest != stage_repo.input_digest()?
        || record.input_digest != planned_input_digest
    {
        return Err(import_corrupt(
            "in-progress export import record is inconsistent",
        ));
    }

    record.recovered_snapshot = Some(recovered_snapshot.clone());
    record.imported_objects = budget.references.len();
    record.imported_bytes = budget.bytes;
    // Refresh import accounting and the staged snapshot handle before any
    // destination CAS or catalog mutation.
    write_import_record(&destination_repo, &record)?;
    copy_stage_objects(&stage_repo, &destination_repo)?;
    validate_destination_state(
        &destination_repo,
        &source_services,
        &selected,
        &identity,
        true,
    )?;
    for capture in &selected {
        write_destination_capture(&destination_repo, capture)?;
    }
    promote_services(&destination_repo, &source_services, &identity)?;
    let destination_digest = destination_repo.input_digest()?;
    if destination_digest != record.input_digest {
        return Err(import_corrupt(
            "promoted service set changed the destination input digest",
        ));
    }
    let destination_manifest =
        Check::load_snapshot_manifest(&destination_repo, &recovered_snapshot)?;
    if destination_manifest.input_digest != destination_digest
        || destination_manifest.service_manifests.len() != selected.len()
    {
        return Err(import_corrupt(
            "recovered snapshot does not match the promoted destination",
        ));
    }
    record.status = "COMPLETE".into();
    record.recovered_snapshot = Some(recovered_snapshot);
    write_import_record(&destination_repo, &record)?;
    cleanup_stage(&destination_repo, &stage_path, &identity)?;
    Ok(result(&record))
}

fn result(record: &ImportRecord) -> Value {
    json!({
        "schema": IMPORT_SCHEMA,
        "status": "RECOVERED",
        "sourceSnapshot": record.source_snapshot,
        "snapshot": record.recovered_snapshot,
        "sourceStoreId": record.source_store_id,
        "recoveredSnapshot": record.recovered_snapshot,
        "sourceAuthority": "RETAINED_SOURCE_NOT_REVERIFIED",
        "captures": record.captures,
        "services": record.services,
        "importedObjects": record.imported_objects,
        "importedBytes": record.imported_bytes,
    })
}

fn read_source_snapshot(
    reader: &ExportReader,
    handle: &str,
    budget: &mut ImportBudget,
) -> Result<(cache::ObjectRef, CheckManifest), ClewError> {
    let (digest, size) = handle
        .rsplit_once('/')
        .ok_or_else(|| invalid("source snapshot must be a sha256:identity/size handle"))?;
    let size = size
        .parse::<u64>()
        .map_err(|_| invalid("source snapshot size is invalid"))?;
    if handle != format!("{digest}/{size}") || size == 0 || size > check::PORTABLE_CACHE_MAX_BYTES {
        return Err(invalid(
            "source snapshot handle is not canonical or exceeds its bound",
        ));
    }
    let reference = cache::ObjectRef::new(check::CHECK_MANIFEST_SCHEMA.into(), digest.into(), size);
    budget.observe(&reference)?;
    let raw = read_required(reader, &reference)?;
    let manifest: CheckManifest = serde_json::from_slice(&raw)
        .map_err(|_| invalid("source snapshot manifest is malformed"))?;
    if manifest.schema != check::CHECK_MANIFEST_SCHEMA
        || manifest.composition.is_some()
        || manifest.dependencies_index.schema != fact_index::FACT_INDEX_SCHEMA
        || manifest.source_inputs.schema != source_inputs::MANIFEST_SCHEMA
        || manifest.service_manifests.is_empty()
        || manifest.service_manifests.len() > store::MAX_RECORDS
    {
        return Err(invalid("source snapshot schema or scope is unsupported"));
    }
    for (id, capture) in &manifest.service_manifests {
        if !store::valid_id(id) || capture.service != *id {
            return Err(invalid(
                "source snapshot contains an invalid service capture",
            ));
        }
        cache::validate_capture(capture)?;
    }
    Ok((reference, manifest))
}

fn read_selected_captures(
    reader: &ExportReader,
    names: &[String],
    snapshot: &CheckManifest,
    budget: &mut ImportBudget,
) -> Result<Vec<SelectedCapture>, ClewError> {
    let mut selected = Vec::new();
    let mut seen_names = BTreeSet::new();
    let mut seen_services = BTreeSet::new();
    for name in names {
        validate_capture_name(name)?;
        if !seen_names.insert(name.clone()) {
            return Err(invalid("capture list contains a duplicate basename"));
        }
        let raw = reader.read_capture(name)?;
        let manifest: cache::CaptureManifest = serde_json::from_slice(&raw)
            .map_err(|_| invalid("selected capture manifest is malformed"))?;
        cache::validate_capture(&manifest)?;
        if !seen_services.insert(manifest.service.clone()) {
            return Err(invalid("export recovery accepts one capture per service"));
        }
        let embedded = snapshot
            .service_manifests
            .get(&manifest.service)
            .ok_or_else(|| invalid("selected capture is not in the source snapshot"))?;
        validate_capture_match(embedded, &manifest)?;
        let manifest_digest = cache::content_digest(&raw);
        budget.observe_file(&manifest_digest, raw.len() as u64)?;
        selected.push(SelectedCapture {
            name: name.clone(),
            raw,
            receipt: CaptureReceipt {
                service: manifest.service.clone(),
                manifest_digest,
                cacheability: manifest.cacheability.clone(),
                reason: manifest.reason.clone(),
            },
            manifest,
        });
    }
    selected.sort_by(|left, right| left.name.cmp(&right.name));
    Ok(selected)
}

fn validate_capture_match(
    embedded: &cache::CaptureManifest,
    selected: &cache::CaptureManifest,
) -> Result<(), ClewError> {
    if embedded.cacheability == cache::NON_CACHEABLE
        && (selected.cacheability != cache::NON_CACHEABLE || selected.reason != embedded.reason)
    {
        return Err(invalid(
            "selected capture changes source non-cacheability authority or reason",
        ));
    }
    let mut normalized = selected.clone();
    normalized.cacheability = embedded.cacheability.clone();
    normalized.reason = embedded.reason.clone();
    if &normalized != embedded {
        return Err(invalid(
            "selected capture differs from the source snapshot outside cacheability metadata",
        ));
    }
    Ok(())
}

fn copy_capture_closure(
    reader: &ExportReader,
    stage: &Repository,
    manifest: &cache::CaptureManifest,
    budget: &mut ImportBudget,
    batch: &mut CopyBatch,
) -> Result<(), ClewError> {
    copy_external_ref(reader, stage, &manifest.sources, budget, batch)?;
    copy_external_ref(reader, stage, &manifest.contracts, budget, batch)?;
    import_fact_index(
        reader,
        stage,
        &manifest.observations,
        &manifest.service,
        budget,
        batch,
    )
}

fn copy_external_ref(
    reader: &ExportReader,
    stage: &Repository,
    reference: &cache::ObjectRef,
    budget: &mut ImportBudget,
    batch: &mut CopyBatch,
) -> Result<Vec<u8>, ClewError> {
    budget.observe(reference)?;
    let payload = read_required(reader, reference)?;
    batch.push(stage, reference, payload.clone())?;
    Ok(payload)
}

fn import_fact_index(
    reader: &ExportReader,
    stage: &Repository,
    reference: &cache::ObjectRef,
    service: &str,
    budget: &mut ImportBudget,
    batch: &mut CopyBatch,
) -> Result<(), ClewError> {
    if reference.schema != fact_index::FACT_INDEX_SCHEMA {
        return Err(import_corrupt(
            "capture observation root has the wrong schema",
        ));
    }
    let raw = copy_external_ref(reader, stage, reference, budget, batch)?;
    let root: fact_index::FactIndexRoot = serde_json::from_slice(&raw)
        .map_err(|_| invalid("capture fact-index root is malformed"))?;
    if root.schema != fact_index::FACT_INDEX_SCHEMA
        || root.protocol != fact_index::FACT_INDEX_SCHEMA
        || root.buckets.len() != fact_index::BUCKETS
        || root.bucket_counts.len() != fact_index::BUCKETS
    {
        return Err(import_corrupt(
            "capture fact-index root schema or bucket shape is invalid",
        ));
    }
    let mut keys = BTreeSet::new();
    for bucket in 0..fact_index::BUCKETS {
        let Some(page_ref) = root.buckets[bucket].as_ref() else {
            if root.bucket_counts[bucket] != 0 {
                return Err(import_corrupt("capture fact-index count has no page"));
            }
            continue;
        };
        if page_ref.schema != fact_index::FACT_PAGE_SCHEMA {
            return Err(import_corrupt(
                "capture fact-index page reference schema is invalid",
            ));
        }
        let raw_page = copy_external_ref(reader, stage, page_ref, budget, batch)?;
        let page: fact_index::FactPage = serde_json::from_slice(&raw_page)
            .map_err(|_| invalid("capture fact-index page is malformed"))?;
        if page.schema != fact_index::FACT_PAGE_SCHEMA
            || page.protocol != fact_index::FACT_INDEX_SCHEMA
            || page.bucket != bucket
            || page.entries.len() as u64 != root.bucket_counts[bucket]
        {
            return Err(import_corrupt(
                "capture fact-index page identity or count is invalid",
            ));
        }
        for entry in &page.entries {
            let canonical = entry.key.canonical();
            if fact_index::import_bucket(&canonical) != bucket
                || !keys.insert(canonical)
                || entry.key.domain != fact_index::OBSERVATION_DOMAIN
                || entry.key.scope != check::CHECK_DEPENDENCIES_SCOPE
                || entry.key.revision != "check-map/1.0"
                || entry.key.source_state != "CHECK_MAP_SLOT_V1"
                || entry.key.repository != service
                || entry.payload.schema != fact_index::OBSERVATION_OBJECT_SCHEMA
            {
                return Err(import_corrupt(
                    "capture fact-index membership is inconsistent",
                ));
            }
            let payload = copy_external_ref(reader, stage, &entry.payload, budget, batch)?;
            let observation: Observation = serde_json::from_slice(&payload)
                .map_err(|_| invalid("capture observation payload is malformed"))?;
            if observation.id != entry.key.semantic
                || observation.service != entry.key.repository
                || observation.kind != entry.kind
                || observation.symbol != entry.symbol
                || digest(&observation.normalized)? != observation.digest
            {
                return Err(import_corrupt(
                    "capture observation does not match its membership",
                ));
            }
        }
    }
    Ok(())
}

fn validate_evidence(
    manifest: &cache::CaptureManifest,
    evidence: &super::model::ServiceEvidence,
    all_source_ids: &mut BTreeSet<String>,
    all_observation_ids: &mut BTreeSet<String>,
    all_entrypoint_ids: &mut BTreeSet<String>,
) -> Result<(), ClewError> {
    if evidence.schema != "codeclew-documentation-service-evidence/1.0"
        || evidence.service != manifest.service
        || evidence.revision != manifest.revision
        || evidence.service_digest != manifest.service_digest
        || evidence.extractor != manifest.extractor
        || evidence.runtime_mode != manifest.runtime_mode
        || evidence.coverage != manifest.coverage
        || evidence.boundaries != manifest.boundaries
        || evidence.entrypoints != manifest.entrypoints
    {
        return Err(import_corrupt(
            "capture evidence does not match its manifest identity",
        ));
    }
    for (id, source) in &evidence.sources {
        if id != &source.id
            || source.service != manifest.service
            || source.revision != manifest.revision
            || cache::content_digest(source.text.as_bytes()) != source.text_digest
            || !all_source_ids.insert(id.clone())
        {
            return Err(import_corrupt(
                "selected source identity, revision, or digest is invalid",
            ));
        }
    }
    for (id, observation) in &evidence.observations {
        if id != &observation.id
            || observation.service != manifest.service
            || observation.source_ids.iter().any(|id| {
                evidence
                    .sources
                    .get(id)
                    .is_none_or(|source| source.service != manifest.service)
            })
            || !all_observation_ids.insert(id.clone())
        {
            return Err(import_corrupt(
                "capture observation identity, source reference, or selected-service uniqueness is invalid",
            ));
        }
    }
    let mut local_entrypoint_ids = BTreeSet::new();
    for entrypoint in &evidence.entrypoints {
        if entrypoint.service != manifest.service
            || entrypoint.id.is_empty()
            || !local_entrypoint_ids.insert(entrypoint.id.clone())
            || !all_entrypoint_ids.insert(entrypoint.id.clone())
            || entrypoint
                .source_ids
                .iter()
                .any(|id| !evidence.sources.contains_key(id))
            || entrypoint
                .dependency_ids
                .iter()
                .any(|id| !evidence.observations.contains_key(id))
        {
            return Err(import_corrupt(
                "capture entrypoint identity or dependency references are inconsistent",
            ));
        }
    }
    Ok(())
}

fn read_required(
    reader: &ExportReader,
    reference: &cache::ObjectRef,
) -> Result<Vec<u8>, ClewError> {
    reader
        .objects
        .read(
            &reference.digest,
            reference.size,
            check::PORTABLE_CACHE_MAX_BYTES,
        )?
        .ok_or_else(|| {
            ClewError::new(
                ErrorCode::StateCorrupt,
                format!(
                    "selected immutable export object is missing: {}",
                    reference.digest
                ),
            )
        })
}

impl ExportReader {
    fn open(path: &Path) -> Result<Self, ClewError> {
        let root = checked_directory(path)?;
        let cache_dir = checked_child_directory(&root, "cache")?;
        let marker = read_regular(&cache_dir.join("object-layout.json"), 4096)?;
        let layout: ExportLayout = serde_json::from_slice(&marker)
            .map_err(|_| invalid("export object-layout marker is malformed"))?;
        if layout.schema != EXPORT_LAYOUT_SCHEMA
            || layout.policy != EXPORT_LAYOUT_POLICY
            || uuid::Uuid::parse_str(&layout.store_id).is_err()
            || layout.database != format!(".codeclew/cache/objects-{}.sqlite3", layout.store_id)
        {
            return Err(import_corrupt("export object-layout marker is unsupported"));
        }
        let database_name = format!("objects-{}.sqlite3", layout.store_id);
        let database = cache_dir.join(&database_name);
        let metadata = fs::symlink_metadata(&database)
            .map_err(|_| import_corrupt("export immutable object database is missing"))?;
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err(import_corrupt(
                "export object database is not a regular file",
            ));
        }
        for suffix in ["-wal", "-shm", "-journal"] {
            let sidecar = cache_dir.join(format!("{database_name}{suffix}"));
            match fs::symlink_metadata(&sidecar) {
                Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
                    return Err(import_corrupt("export SQLite sidecar is unsafe"));
                }
                Ok(metadata) if metadata.len() != 0 => {
                    return Err(import_corrupt(
                        "export has a nonempty SQLite WAL or journal; stop and export the store before recovery",
                    ));
                }
                Ok(_) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(_) => return Err(import_corrupt("export SQLite sidecar cannot be inspected")),
            }
        }
        let objects = ImmutableSqliteObjects::open(&database, &layout.store_id)?;
        Ok(Self {
            root,
            store_id: layout.store_id,
            objects,
        })
    }

    fn read_capture(&self, name: &str) -> Result<Vec<u8>, ClewError> {
        validate_capture_name(name)?;
        let cache_dir = checked_child_directory(&self.root, "cache")?;
        read_regular(&cache_dir.join(name), store::MAX_RECORD)
    }
}

fn validate_capture_name(name: &str) -> Result<(), ClewError> {
    store::relative(name)?;
    if name.is_empty()
        || name.len() > 240
        || Path::new(name).components().count() != 1
        || !name.ends_with(".json")
        || matches!(
            name,
            "object-layout.json" | "snapshot-import.json" | "latest-check.json"
        )
        || name.starts_with("objects-")
    {
        return Err(invalid("capture must be a safe explicit manifest basename"));
    }
    Ok(())
}

fn checked_directory(path: &Path) -> Result<PathBuf, ClewError> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir().map_err(super::io_error)?.join(path)
    };
    let mut current = PathBuf::new();
    for component in absolute.components() {
        match component {
            Component::Prefix(prefix) => current.push(prefix.as_os_str()),
            Component::RootDir => current.push(component.as_os_str()),
            Component::CurDir => {}
            Component::ParentDir => {
                return Err(invalid("export path cannot contain parent traversal"));
            }
            Component::Normal(segment) => {
                current.push(segment);
                let metadata = fs::symlink_metadata(&current)
                    .map_err(|_| invalid("export directory is missing or unreadable"))?;
                if metadata.file_type().is_symlink() || !metadata.is_dir() {
                    return Err(invalid("export path contains a symlink or non-directory"));
                }
            }
        }
    }
    absolute.canonicalize().map_err(super::io_error)
}

fn checked_child_directory(root: &Path, name: &str) -> Result<PathBuf, ClewError> {
    let path = root.join(name);
    let metadata = fs::symlink_metadata(&path)
        .map_err(|_| invalid("export cache directory is missing or unreadable"))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(invalid("export cache path is not a safe directory"));
    }
    Ok(path)
}

fn read_regular(path: &Path, limit: u64) -> Result<Vec<u8>, ClewError> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK)
        .open(path)
        .map_err(|_| invalid("export file cannot be opened safely"))?;
    read_bounded_file(file, limit)
}

fn read_bounded_file(file: File, limit: u64) -> Result<Vec<u8>, ClewError> {
    let metadata = file.metadata().map_err(super::io_error)?;
    if !metadata.is_file() || metadata.len() > limit {
        return Err(invalid("export file is not a bounded regular file"));
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    file.take(limit.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(super::io_error)?;
    if bytes.len() as u64 > limit {
        return Err(invalid("export file grew beyond its size bound"));
    }
    Ok(bytes)
}

fn reject_path_overlap(export: &Path, destination: &Path) -> Result<(), ClewError> {
    if export == destination || export.starts_with(destination) || destination.starts_with(export) {
        return Err(invalid("export and destination paths must not overlap"));
    }
    Ok(())
}

fn validate_destination_paths(root: &Path) -> Result<(), ClewError> {
    for directory in [
        ".codeclew",
        ".codeclew/cache",
        "catalog",
        "catalog/services",
        "docs",
    ] {
        ensure_real_directory(
            &root.join(directory),
            "destination contains an unsafe directory",
        )?;
    }
    ensure_regular_file(
        &root.join("codeclew-docs.yaml"),
        "destination manifest is unsafe",
    )?;
    let marker_path = root.join(".codeclew/cache/object-layout.json");
    ensure_regular_file(&marker_path, "destination object-store marker is unsafe")?;
    let layout: ExportLayout = serde_json::from_slice(&read_regular(&marker_path, 4096)?)
        .map_err(|_| import_corrupt("destination object-store marker is malformed"))?;
    if layout.schema != EXPORT_LAYOUT_SCHEMA
        || layout.policy != EXPORT_LAYOUT_POLICY
        || uuid::Uuid::parse_str(&layout.store_id).is_err()
        || layout.database != format!(".codeclew/cache/objects-{}.sqlite3", layout.store_id)
    {
        return Err(import_corrupt(
            "destination object-store marker is unsupported",
        ));
    }
    ensure_regular_file(
        &root.join(&layout.database),
        "destination object database is unsafe",
    )?;
    Ok(())
}

fn ensure_real_directory(path: &Path, message: &'static str) -> Result<(), ClewError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if !metadata.file_type().is_symlink() && metadata.is_dir() => Ok(()),
        _ => Err(import_corrupt(message)),
    }
}

fn ensure_regular_file(path: &Path, message: &'static str) -> Result<(), ClewError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if !metadata.file_type().is_symlink() && metadata.is_file() => Ok(()),
        _ => Err(import_corrupt(message)),
    }
}

fn import_corrupt(message: &'static str) -> ClewError {
    ClewError::new(ErrorCode::StateCorrupt, message)
}

fn import_corrupt_ref(message: &str, digest: &str) -> ClewError {
    ClewError::new(ErrorCode::StateCorrupt, format!("{message}: {digest}"))
}

fn read_import_record(repo: &Repository) -> Result<Option<ImportRecord>, ClewError> {
    let path = repo.path(IMPORT_RECORD_PATH)?;
    let metadata = match fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(import_corrupt("snapshot import record cannot be inspected")),
    };
    if metadata.file_type().is_symlink()
        || !metadata.is_file()
        || metadata.len() > MAX_IMPORT_RECORD_BYTES
    {
        return Err(import_corrupt(
            "snapshot import record is not a bounded regular file",
        ));
    }
    let data = read_regular(&path, MAX_IMPORT_RECORD_BYTES)?;
    let record: ImportRecord = serde_json::from_slice(&data)
        .map_err(|_| import_corrupt("snapshot import record is malformed"))?;
    validate_import_record(&record)?;
    Ok(Some(record))
}

fn serialized_import_record(record: &ImportRecord) -> Result<Vec<u8>, ClewError> {
    let data = bytes(record)?;
    if data.len() as u64 > MAX_IMPORT_RECORD_BYTES {
        return Err(ClewError::new(
            ErrorCode::ResourceLimit,
            "snapshot import receipt exceeds its portable record budget",
        ));
    }
    Ok(data)
}

fn ensure_import_record_capacity(record: &ImportRecord) -> Result<(), ClewError> {
    let mut probe = record.clone();
    probe.status = "IN_PROGRESS".into();
    probe.recovered_snapshot = Some(format!("sha256:{}/{}", "0".repeat(64), u64::MAX));
    probe.imported_objects = usize::MAX;
    probe.imported_bytes = u64::MAX;
    serialized_import_record(&probe)?;
    Ok(())
}

fn write_import_record(repo: &Repository, record: &ImportRecord) -> Result<(), ClewError> {
    validate_import_record(record)?;
    let data = serialized_import_record(record)?;
    repo.atomic(IMPORT_RECORD_PATH, &data)
}

fn validate_import_record(record: &ImportRecord) -> Result<(), ClewError> {
    if record.schema != IMPORT_RECORD_SCHEMA
        || !matches!(record.status.as_str(), "IN_PROGRESS" | "COMPLETE")
        || record.source_store_id.is_empty()
        || record.source_snapshot.is_empty()
        || record.input_digest.is_empty()
        || record.captures.is_empty()
        || record.captures.len() > store::MAX_RECORDS
        || record.services.is_empty()
        || record.services.len() > store::MAX_RECORDS
        || (record.status == "COMPLETE" && record.recovered_snapshot.is_none())
    {
        return Err(import_corrupt("snapshot import record identity is invalid"));
    }
    for (name, capture) in &record.captures {
        validate_capture_name(name)
            .map_err(|_| import_corrupt("snapshot import capture name is unsafe"))?;
        if !store::valid_id(&capture.service) || capture.manifest_digest.is_empty() {
            return Err(import_corrupt("snapshot import capture record is invalid"));
        }
    }
    for id in record.services.keys() {
        if !store::valid_id(id) {
            return Err(import_corrupt(
                "snapshot import service identity is invalid",
            ));
        }
    }
    Ok(())
}

fn stage_path(repo: &Repository, identity: &str) -> Result<PathBuf, ClewError> {
    let token = stage_token(identity)?;
    repo.path(&format!(".codeclew/cache/.snapshot-import-{token}"))
}

fn stage_token(identity: &str) -> Result<&str, ClewError> {
    let token = identity.strip_prefix("sha256:").unwrap_or(identity);
    if token.len() != 64 || !token.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(invalid("snapshot import identity is malformed"));
    }
    Ok(token)
}

fn stage_marker(identity: &str) -> Result<Vec<u8>, ClewError> {
    bytes(&json!({"schema":STAGE_SCHEMA,"identity":identity}))
}

fn cleanup_marker(identity: &str) -> Result<Vec<u8>, ClewError> {
    bytes(&json!({"schema":STAGE_CLEANUP_SCHEMA,"identity":identity}))
}

fn open_or_create_stage(
    destination: &Repository,
    path: &Path,
    identity: &str,
) -> Result<Repository, ClewError> {
    install_owned_stage_root(destination, path, identity)?;
    let work = path.join("work");
    match fs::symlink_metadata(&work) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
            return Err(import_corrupt("snapshot import work path is unsafe"));
        }
        Ok(_) => {
            let manifest = work.join("codeclew-docs.yaml");
            match fs::symlink_metadata(&manifest) {
                Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
                    return Err(import_corrupt("snapshot import work manifest is unsafe"));
                }
                Ok(_) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(_) => return Err(import_corrupt("snapshot import work is unreadable")),
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => {
            return Err(import_corrupt(
                "snapshot import work path cannot be inspected",
            ));
        }
    }
    Repository::init(&work, &destination.manifest.title)?;
    Repository::open(&work)
}

fn install_owned_stage_root(
    destination: &Repository,
    path: &Path,
    identity: &str,
) -> Result<(), ClewError> {
    let token = identity.strip_prefix("sha256:").unwrap_or(identity);
    if token.len() != 64 || !token.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(invalid("snapshot import identity is malformed"));
    }
    let parent = path
        .parent()
        .ok_or_else(|| import_corrupt("snapshot import staging parent is missing"))?;
    ensure_real_directory(parent, "snapshot import staging parent is unsafe")?;
    let prepared = parent.join(format!(".snapshot-import-{token}.prepare"));
    let marker = stage_marker(identity)?;
    let final_exists = match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
            return Err(import_corrupt("snapshot import staging path is unsafe"));
        }
        Ok(_) => true,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
        Err(_) => {
            return Err(import_corrupt(
                "snapshot import staging path cannot be inspected",
            ));
        }
    };

    match fs::symlink_metadata(&prepared) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
            return Err(import_corrupt("snapshot import preparation path is unsafe"));
        }
        Ok(_) => {
            remove_owned_atomic_temps(&prepared)?;
            let entries = fs::read_dir(&prepared)
                .map_err(|_| import_corrupt("snapshot import preparation path is unreadable"))?
                .collect::<Result<Vec<_>, _>>()
                .map_err(super::io_error)?;
            if entries.is_empty() {
                fs::remove_dir(&prepared).map_err(super::io_error)?;
            } else if entries.len() == 1 && entries[0].file_name() == "stage.json" {
                if read_regular(&prepared.join("stage.json"), 4096)? != marker {
                    return Err(ClewError::new(
                        ErrorCode::WwConflict,
                        "snapshot import preparation path belongs to another operation",
                    ));
                }
                if final_exists {
                    fs::remove_file(prepared.join("stage.json")).map_err(super::io_error)?;
                    fs::remove_dir(&prepared).map_err(super::io_error)?;
                } else {
                    fs::rename(&prepared, path).map_err(super::io_error)?;
                    sync_directory(parent)?;
                }
            } else {
                return Err(import_corrupt(
                    "snapshot import preparation path contains unexpected files",
                ));
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => {
            return Err(import_corrupt(
                "snapshot import preparation path is unreadable",
            ));
        }
    }

    if !path_presence(path)? {
        fs::create_dir(&prepared).map_err(super::io_error)?;
        let relative = prepared
            .join("stage.json")
            .strip_prefix(&destination.root)
            .map_err(|_| import_corrupt("snapshot import preparation path escaped destination"))?
            .to_str()
            .ok_or_else(|| import_corrupt("snapshot import preparation path is not UTF-8"))?
            .to_owned();
        destination.atomic(&relative, &marker)?;
        sync_directory(&prepared)?;
        fs::rename(&prepared, path).map_err(super::io_error)?;
        sync_directory(parent)?;
    }

    ensure_real_directory(path, "snapshot import staging path is unsafe")?;
    if read_regular(&path.join("stage.json"), 4096)? != marker {
        return Err(ClewError::new(
            ErrorCode::WwConflict,
            "snapshot import staging path belongs to another operation",
        ));
    }
    Ok(())
}

fn remove_owned_atomic_temps(directory: &Path) -> Result<(), ClewError> {
    for entry in fs::read_dir(directory).map_err(super::io_error)? {
        let entry = entry.map_err(super::io_error)?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if !name.starts_with(".tmp") {
            continue;
        }
        let file_type = entry.file_type().map_err(super::io_error)?;
        if !file_type.is_file() {
            return Err(import_corrupt("owned atomic temporary path is unsafe"));
        }
        fs::remove_file(entry.path()).map_err(super::io_error)?;
    }
    Ok(())
}

fn sync_directory(path: &Path) -> Result<(), ClewError> {
    File::open(path)
        .and_then(|directory| directory.sync_all())
        .map_err(super::io_error)
}

fn cleanup_stage(repo: &Repository, path: &Path, identity: &str) -> Result<(), ClewError> {
    let token = stage_token(identity)?;
    let parent = path
        .parent()
        .ok_or_else(|| import_corrupt("snapshot import staging parent is missing"))?;
    let cleanup_path = parent.join(format!(".snapshot-import-{token}.cleanup"));
    let cleanup_name = format!(".codeclew/cache/.snapshot-import-{token}.cleanup.json");
    let cleanup_marker_path = repo.path(&cleanup_name)?;
    let cleanup_bytes = cleanup_marker(identity)?;
    let cleanup_intent = match fs::symlink_metadata(&cleanup_marker_path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
            return Err(import_corrupt("snapshot import cleanup marker is unsafe"));
        }
        Ok(_) => {
            let current = read_regular(&cleanup_marker_path, 4096)?;
            if current != cleanup_bytes {
                return Err(import_corrupt("snapshot import cleanup marker changed"));
            }
            true
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
        Err(_) => {
            return Err(import_corrupt(
                "snapshot import cleanup marker cannot be inspected",
            ));
        }
    };
    let cleanup_exists = match fs::symlink_metadata(&cleanup_path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
            return Err(import_corrupt("snapshot import cleanup path is unsafe"));
        }
        Ok(_) => true,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
        Err(_) => {
            return Err(import_corrupt(
                "snapshot import cleanup path cannot be inspected",
            ));
        }
    };
    if cleanup_exists {
        if !cleanup_intent || path_presence(path)? {
            return Err(import_corrupt(
                "snapshot import cleanup path has no exclusive ownership proof",
            ));
        }
        fs::remove_dir_all(&cleanup_path).map_err(super::io_error)?;
        sync_directory(parent)?;
    } else if path_presence(path)? {
        ensure_real_directory(path, "snapshot import staging path is unsafe")?;
        if read_regular(&path.join("stage.json"), 4096)? != stage_marker(identity)? {
            return Err(import_corrupt(
                "refusing to remove an unowned import staging path",
            ));
        }
        if !cleanup_intent {
            repo.atomic(&cleanup_name, &cleanup_bytes)?;
        }
        fs::rename(path, &cleanup_path).map_err(super::io_error)?;
        sync_directory(parent)?;
        fs::remove_dir_all(&cleanup_path).map_err(super::io_error)?;
        sync_directory(parent)?;
    }
    if cleanup_intent {
        fs::remove_file(cleanup_marker_path).map_err(super::io_error)?;
        sync_directory(parent)?;
    }
    Ok(())
}

fn path_presence(path: &Path) -> Result<bool, ClewError> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(_) => Err(import_corrupt("snapshot import path cannot be inspected")),
    }
}

fn ensure_stage_services(
    stage: &Repository,
    selected: &BTreeMap<String, Service>,
    identity: &str,
) -> Result<(), ClewError> {
    let inputs = stage.inputs()?;
    if !inputs.services.is_empty() {
        if &inputs.services != selected
            || !inputs.interactions.is_empty()
            || !inputs.scenarios.is_empty()
            || !inputs.entities.is_empty()
            || !inputs.notes.is_empty()
        {
            return Err(import_corrupt(
                "snapshot import staging root has unexpected inputs",
            ));
        }
        return Ok(());
    }
    if stage
        .root
        .join(".codeclew/cache/latest-check.json")
        .exists()
    {
        return Err(import_corrupt(
            "snapshot import staging root has unexpected latest evidence",
        ));
    }
    promote_services(stage, selected, identity)?;
    if stage.inputs()?.services != *selected {
        return Err(import_corrupt(
            "snapshot import staging service set did not persist",
        ));
    }
    Ok(())
}

fn write_capture(stage: &Repository, capture: &SelectedCapture) -> Result<(), ClewError> {
    write_exact(
        stage,
        &format!(".codeclew/cache/{}", capture.name),
        &capture.raw,
    )
}

fn write_destination_capture(
    repo: &Repository,
    capture: &SelectedCapture,
) -> Result<(), ClewError> {
    write_exact(
        repo,
        &format!(".codeclew/cache/{}", capture.name),
        &capture.raw,
    )
}

fn write_exact(repo: &Repository, relative: &str, contents: &[u8]) -> Result<(), ClewError> {
    let path = repo.path(relative)?;
    match fs::symlink_metadata(&path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
            Err(import_corrupt("capture destination is not a regular file"))
        }
        Ok(_) => {
            if fs::read(&path).map_err(super::io_error)? == contents {
                Ok(())
            } else {
                Err(ClewError::new(
                    ErrorCode::WwConflict,
                    "destination capture basename already contains different bytes",
                ))
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            repo.atomic(relative, contents)
        }
        Err(_) => Err(import_corrupt("capture destination cannot be inspected")),
    }
}

fn copy_stage_objects(stage: &Repository, destination: &Repository) -> Result<(), ClewError> {
    let owned = cache::owned_objects(stage, usize::MAX)?;
    cache::with_read_session(stage, |session| {
        let mut payloads = Vec::<Vec<u8>>::new();
        let mut batch_bytes = 0usize;
        for (digest, size) in owned {
            let reference =
                cache::ObjectRef::new("snapshot-import-copy/1.0".into(), digest.clone(), size);
            let payload = session
                .get(&reference, check::PORTABLE_CACHE_MAX_BYTES)
                .map_err(|_| import_corrupt_ref("staged object could not be verified", &digest))?
                .ok_or_else(|| import_corrupt_ref("staged object is missing", &digest))?;
            if !payloads.is_empty()
                && (payloads.len() >= COPY_BATCH_OBJECTS
                    || batch_bytes.saturating_add(payload.len()) > COPY_BATCH_BYTES)
            {
                let slices = payloads.iter().map(Vec::as_slice).collect::<Vec<_>>();
                cache::put_batch(destination, "snapshot-import-copy/1.0", &slices)?;
                payloads.clear();
                batch_bytes = 0;
            }
            batch_bytes = batch_bytes.saturating_add(payload.len());
            payloads.push(payload);
            if batch_bytes >= COPY_BATCH_BYTES || payloads.len() >= COPY_BATCH_OBJECTS {
                let slices = payloads.iter().map(Vec::as_slice).collect::<Vec<_>>();
                cache::put_batch(destination, "snapshot-import-copy/1.0", &slices)?;
                payloads.clear();
                batch_bytes = 0;
            }
        }
        if !payloads.is_empty() {
            let slices = payloads.iter().map(Vec::as_slice).collect::<Vec<_>>();
            cache::put_batch(destination, "snapshot-import-copy/1.0", &slices)?;
        }
        Ok(())
    })
}

fn validate_empty_destination(repo: &Repository) -> Result<(), ClewError> {
    validate_unclaimed_destination(repo)?;
    match cache::owned_objects(repo, 0) {
        Ok(objects) if objects.is_empty() => {}
        Ok(_) => {
            return Err(ClewError::new(
                ErrorCode::WwConflict,
                "destination already contains immutable documentation objects",
            ));
        }
        Err(error) if error.code == ErrorCode::ResourceLimit => {
            return Err(ClewError::new(
                ErrorCode::WwConflict,
                "destination already contains immutable documentation objects",
            ));
        }
        Err(error) => return Err(error),
    }
    Ok(())
}

fn validate_unclaimed_destination(repo: &Repository) -> Result<(), ClewError> {
    let inputs = repo.inputs()?;
    if !inputs.services.is_empty()
        || !inputs.interactions.is_empty()
        || !inputs.scenarios.is_empty()
        || !inputs.process_states.is_empty()
        || !inputs.entities.is_empty()
        || !inputs.notes.is_empty()
        || !inputs.evidence_expectations.is_empty()
        || !inputs.update_policies.is_empty()
        || !inputs.update_state.targets.is_empty()
    {
        return Err(ClewError::new(
            ErrorCode::WwConflict,
            "destination has existing documentation inputs",
        ));
    }
    if repo.path(".codeclew/cache/latest-check.json")?.exists()
        || repo.path("docs/index.html")?.exists()
    {
        return Err(ClewError::new(
            ErrorCode::WwConflict,
            "destination has existing check or published reader state",
        ));
    }
    for directory in [
        "catalog/services",
        "catalog/interactions",
        "scenarios",
        "narratives",
        "evidence",
        ".codeclew/bindings",
    ] {
        if !directory_empty(&repo.path(directory)?)? {
            return Err(ClewError::new(
                ErrorCode::WwConflict,
                "destination contains existing documentation records",
            ));
        }
    }
    for directory in [
        ".codeclew/accounts",
        ".codeclew/execution",
        ".codeclew/evidence",
        ".codeclew/job-inputs",
        ".codeclew/job-results",
        ".codeclew/jobs",
        ".codeclew/proposals",
        ".codeclew/work",
        ".codeclew/cache/pins",
    ] {
        if !directory_empty(&repo.path(directory)?)? {
            return Err(ClewError::new(
                ErrorCode::WwConflict,
                "destination contains private job, execution, or retained state",
            ));
        }
    }
    validate_reader_directory(repo)
}

fn validate_destination_state(
    repo: &Repository,
    selected_services: &BTreeMap<String, Service>,
    captures: &[SelectedCapture],
    identity: &str,
    allow_imported: bool,
) -> Result<(), ClewError> {
    let inputs = repo.inputs()?;
    let has_services = !inputs.services.is_empty();
    if has_services {
        if !allow_imported
            || &inputs.services != selected_services
            || !inputs.interactions.is_empty()
            || !inputs.scenarios.is_empty()
            || !inputs.process_states.is_empty()
            || !inputs.entities.is_empty()
            || !inputs.notes.is_empty()
            || !inputs.evidence_expectations.is_empty()
            || !inputs.update_policies.is_empty()
            || !inputs.update_state.targets.is_empty()
        {
            return Err(ClewError::new(
                ErrorCode::WwConflict,
                "destination changed during snapshot import",
            ));
        }
    } else {
        // A matching IN_PROGRESS receipt may already own copied immutable
        // objects; only the no-receipt admission path requires an empty CAS.
        validate_unclaimed_destination(repo)?;
    }
    if repo.path(".codeclew/cache/latest-check.json")?.exists()
        || repo.path("docs/index.html")?.exists()
    {
        return Err(ClewError::new(
            ErrorCode::WwConflict,
            "destination acquired newer reader or latest-check state",
        ));
    }
    if has_services {
        for capture in captures {
            let path = repo.path(&format!(".codeclew/cache/{}", capture.name))?;
            match fs::symlink_metadata(&path) {
                Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
                    return Err(import_corrupt("destination capture path is unsafe"));
                }
                Ok(_) if read_regular(&path, store::MAX_RECORD)? != capture.raw => {
                    return Err(ClewError::new(
                        ErrorCode::WwConflict,
                        "destination capture changed during snapshot import",
                    ));
                }
                Ok(_) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(_) => return Err(import_corrupt("destination capture cannot be inspected")),
            }
        }
    }
    let selected_names = captures
        .iter()
        .map(|capture| capture.name.clone())
        .collect();
    validate_cache_inventory(repo, &selected_names, Some(identity), true)?;
    validate_reader_directory(repo)
}

fn validate_cache_inventory(
    repo: &Repository,
    capture_names: &BTreeSet<String>,
    identity: Option<&str>,
    allow_receipt: bool,
) -> Result<(), ClewError> {
    let cache_dir = repo.path(".codeclew/cache")?;
    ensure_real_directory(&cache_dir, "destination cache directory is unsafe")?;
    if allow_receipt {
        // With an exact import receipt in hand, `.tmp*` files are leftovers
        // from this import's atomic record/capture writes, not user records.
        remove_owned_atomic_temps(&cache_dir)?;
    }
    let marker_path = cache_dir.join("object-layout.json");
    let layout: ExportLayout = serde_json::from_slice(&read_regular(&marker_path, 4096)?)
        .map_err(|_| import_corrupt("destination object-store marker is malformed"))?;
    if layout.schema != EXPORT_LAYOUT_SCHEMA
        || layout.policy != EXPORT_LAYOUT_POLICY
        || uuid::Uuid::parse_str(&layout.store_id).is_err()
        || layout.database != format!(".codeclew/cache/objects-{}.sqlite3", layout.store_id)
    {
        return Err(import_corrupt(
            "destination object-store marker is unsupported",
        ));
    }
    let database = format!("objects-{}.sqlite3", layout.store_id);
    let mut allowed_files = BTreeSet::from([
        "object-layout.json".to_owned(),
        "object-store.lock".to_owned(),
        database.clone(),
        format!("{database}-wal"),
        format!("{database}-shm"),
        format!("{database}-journal"),
    ]);
    allowed_files.extend(capture_names.iter().cloned());
    if allow_receipt {
        allowed_files.insert("snapshot-import.json".into());
    }
    let mut allowed_directories = BTreeSet::new();
    if let Some(identity) = identity {
        let stage = stage_path(repo, identity)?;
        allowed_directories.insert(
            stage
                .file_name()
                .and_then(|name| name.to_str())
                .ok_or_else(|| import_corrupt("snapshot import stage name is invalid"))?
                .to_owned(),
        );
        let token = stage_token(identity)?;
        allowed_directories.insert(format!(".snapshot-import-{token}.prepare"));
        allowed_directories.insert(format!(".snapshot-import-{token}.cleanup"));
        allowed_files.insert(format!(".snapshot-import-{token}.cleanup.json"));
    }
    for entry in fs::read_dir(&cache_dir).map_err(super::io_error)? {
        let entry = entry.map_err(super::io_error)?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let file_type = entry.file_type().map_err(super::io_error)?;
        if file_type.is_file() {
            if !allowed_files.contains(&name) {
                return Err(ClewError::new(
                    ErrorCode::WwConflict,
                    "destination cache contains unrelated files",
                ));
            }
        } else if file_type.is_dir() {
            if !allowed_directories.contains(&name) {
                return Err(ClewError::new(
                    ErrorCode::WwConflict,
                    "destination cache contains unrelated directories",
                ));
            }
        } else {
            return Err(import_corrupt("destination cache contains an unsafe entry"));
        }
    }
    Ok(())
}

fn directory_empty(path: &Path) -> Result<bool, ClewError> {
    match fs::read_dir(path) {
        Ok(mut entries) => Ok(entries.next().is_none()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(true),
        Err(_) => Err(import_corrupt("destination directory cannot be inspected")),
    }
}

fn validate_reader_directory(repo: &Repository) -> Result<(), ClewError> {
    let docs = repo.path("docs")?;
    let allowed = BTreeSet::from(["help.html", "runbooks.html", "catalog.html"]);
    for entry in fs::read_dir(&docs).map_err(super::io_error)? {
        let entry = entry.map_err(super::io_error)?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let metadata = entry.file_type().map_err(super::io_error)?;
        if !metadata.is_file() || metadata.is_symlink() || !allowed.contains(name.as_ref()) {
            return Err(ClewError::new(
                ErrorCode::WwConflict,
                "destination contains existing reader material",
            ));
        }
    }
    Ok(())
}

fn promote_services(
    repo: &Repository,
    services: &BTreeMap<String, Service>,
    identity: &str,
) -> Result<(), ClewError> {
    let target = repo.path("catalog/services")?;
    ensure_real_directory(
        target
            .parent()
            .ok_or_else(|| import_corrupt("service catalog parent is missing"))?,
        "service catalog parent is unsafe",
    )?;
    if directory_has_expected_services(&target, services)? {
        return Ok(());
    }
    if !directory_empty(&target)? {
        return Err(ClewError::new(
            ErrorCode::WwConflict,
            "destination service directory is not empty",
        ));
    }
    let staging_relative = format!(
        ".codeclew/cache/.snapshot-import-{}/promotion/services",
        stage_token(identity)?
    );
    let staging = repo.path(&staging_relative)?;
    fs::create_dir_all(&staging).map_err(super::io_error)?;
    ensure_real_directory(&staging, "staged service promotion path is unsafe")?;
    remove_owned_atomic_temps(&staging)?;

    for entry in fs::read_dir(&staging).map_err(super::io_error)? {
        let entry = entry.map_err(super::io_error)?;
        if !entry.file_type().map_err(super::io_error)?.is_file() {
            return Err(import_corrupt(
                "staged service promotion contains an unsafe file",
            ));
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        let Some(id) = name.strip_suffix(".json") else {
            return Err(import_corrupt(
                "staged service promotion contains an unexpected file",
            ));
        };
        let Some(expected) = services.get(id) else {
            return Err(import_corrupt(
                "staged service promotion contains an unselected service",
            ));
        };
        let actual: Service =
            serde_json::from_slice(&read_regular(&entry.path(), store::MAX_RECORD)?)
                .map_err(|_| import_corrupt("staged service promotion record is malformed"))?;
        if &actual != expected || actual.id != id {
            return Err(import_corrupt(
                "staged service promotion differs from selected services",
            ));
        }
    }
    for (id, service) in services {
        let relative = format!("{staging_relative}/{id}.json");
        let path = repo.path(&relative)?;
        if path.exists() {
            ensure_regular_file(&path, "staged service declaration is unsafe")?;
            let actual: Service = serde_json::from_slice(&read_regular(&path, store::MAX_RECORD)?)
                .map_err(|_| import_corrupt("staged service promotion record is malformed"))?;
            if actual != *service {
                return Err(import_corrupt(
                    "staged service promotion differs from selected services",
                ));
            }
        } else {
            repo.atomic(&relative, &bytes(service)?)?;
        }
    }
    if !directory_has_expected_services(&staging, services)? {
        return Err(import_corrupt(
            "staged service promotion differs from selected services",
        ));
    }
    sync_directory(&staging)?;
    fs::rename(&staging, &target).map_err(super::io_error)?;
    if let Some(parent) = target.parent() {
        File::open(parent)
            .and_then(|directory| directory.sync_all())
            .map_err(super::io_error)?;
    }
    Ok(())
}

fn directory_has_expected_services(
    path: &Path,
    expected: &BTreeMap<String, Service>,
) -> Result<bool, ClewError> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(_) => return Err(import_corrupt("service directory cannot be inspected")),
    };
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(import_corrupt("service directory is unsafe"));
    }
    let mut found = BTreeMap::new();
    for entry in fs::read_dir(path).map_err(super::io_error)? {
        let entry = entry.map_err(super::io_error)?;
        if !entry.file_type().map_err(super::io_error)?.is_file() {
            return Err(import_corrupt("service directory contains a non-file"));
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        let Some(id) = name.strip_suffix(".json") else {
            return Ok(false);
        };
        let service: Service =
            serde_json::from_slice(&read_regular(&entry.path(), store::MAX_RECORD)?)
                .map_err(|_| import_corrupt("service declaration in destination is malformed"))?;
        if service.id != id || found.insert(id.to_owned(), service).is_some() {
            return Err(import_corrupt(
                "service directory contains a mismatched record",
            ));
        }
    }
    Ok(&found == expected)
}

fn verify_completed_import(
    repo: &Repository,
    record: &ImportRecord,
    services: &BTreeMap<String, Service>,
    captures: &[SelectedCapture],
    identity: &str,
) -> Result<(), ClewError> {
    if record.status != "COMPLETE" || repo.inputs()?.services != *services {
        return Err(import_corrupt(
            "completed snapshot import no longer matches its receipt",
        ));
    }
    if repo.input_digest()? != record.input_digest {
        return Err(import_corrupt(
            "completed snapshot import input digest changed",
        ));
    }
    for capture in captures {
        let path = repo.path(&format!(".codeclew/cache/{}", capture.name))?;
        if read_regular(&path, store::MAX_RECORD)? != capture.raw {
            return Err(import_corrupt(
                "completed snapshot import capture bytes changed",
            ));
        }
    }
    let snapshot = record
        .recovered_snapshot
        .as_deref()
        .ok_or_else(|| import_corrupt("completed snapshot import has no recovered snapshot"))?;
    let selected_names = captures
        .iter()
        .map(|capture| capture.name.clone())
        .collect();
    validate_cache_inventory(repo, &selected_names, Some(identity), true)?;
    let expected_manifests = captures
        .iter()
        .map(|capture| (capture.manifest.service.clone(), capture.manifest.clone()))
        .collect::<BTreeMap<_, _>>();
    let saved_manifest = Check::load_snapshot_manifest(repo, snapshot).map_err(|error| {
        ClewError::new(
            ErrorCode::StateCorrupt,
            format!(
                "completed recovered snapshot manifest failed validation: {}",
                error.message
            ),
        )
    })?;
    if saved_manifest.service_manifests != expected_manifests {
        return Err(import_corrupt(
            "completed recovered snapshot capture envelopes differ from selected source captures",
        ));
    }
    let checked = Check::load_snapshot(repo, snapshot).map_err(|error| {
        ClewError::new(
            ErrorCode::StateCorrupt,
            format!(
                "completed recovered snapshot closure failed validation: {}",
                error.message
            ),
        )
    })?;
    let current_inputs = repo.inputs()?;
    let source_inputs = checked.source_inputs.as_ref().ok_or_else(|| {
        import_corrupt("completed recovered snapshot has no source-input closure")
    })?;
    let expected_services = captures
        .iter()
        .map(|capture| capture.manifest.service.clone())
        .collect::<BTreeSet<_>>();
    if checked.input_digest != record.input_digest
        || source_inputs.input_digest != record.input_digest
        || source_inputs.inputs != current_inputs
        || !source_inputs.selected_services.is_empty()
        || source_inputs.retained_services != expected_services
        || checked.services.keys().cloned().collect::<BTreeSet<_>>() != expected_services
    {
        return Err(import_corrupt(
            "completed recovered snapshot no longer matches its selected source inputs",
        ));
    }
    let mut source_ids = BTreeSet::new();
    let mut observation_ids = BTreeSet::new();
    let mut entrypoint_ids = BTreeSet::new();
    for capture in captures {
        let evidence = checked
            .services
            .get(&capture.manifest.service)
            .ok_or_else(|| {
                import_corrupt_ref(
                    "completed recovered snapshot is missing a selected service",
                    &capture.manifest.observations.digest,
                )
            })?;
        validate_evidence(
            &capture.manifest,
            evidence,
            &mut source_ids,
            &mut observation_ids,
            &mut entrypoint_ids,
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;

    fn service(id: &str) -> Service {
        serde_json::from_value(json!({
            "schema":"codeclew-documentation-service/1.0",
            "id":id,
            "title":id,
            "repositoryId":id,
            "repository":format!("https://example.invalid/{id}"),
            "language":"java",
            "profile":"java-17plus-maven-read-only",
            "compilations":[":/main"],
            "targetRef":"main"
        }))
        .unwrap()
    }

    fn identity() -> String {
        format!("sha256:{}", "a".repeat(64))
    }

    #[test]
    fn interrupted_stage_marker_and_service_promotion_resume_atomically() {
        let temporary = tempfile::tempdir().unwrap();
        Repository::init(temporary.path(), "Import stage").unwrap();
        let repo = Repository::open(temporary.path()).unwrap();
        let identity = identity();
        let stage = stage_path(&repo, &identity).unwrap();
        let prepared = stage.parent().unwrap().join(format!(
            ".snapshot-import-{}.prepare",
            stage_token(&identity).unwrap()
        ));
        fs::create_dir(&prepared).unwrap();
        fs::write(prepared.join(".tmp-interrupted"), b"partial atomic marker").unwrap();

        let _stage_repo = open_or_create_stage(&repo, &stage, &identity).unwrap();
        assert_eq!(
            read_regular(&stage.join("stage.json"), 4096).unwrap(),
            stage_marker(&identity).unwrap()
        );
        assert!(!prepared.exists());

        let services = BTreeMap::from([
            ("orders".to_owned(), service("orders")),
            ("inventory".to_owned(), service("inventory")),
        ]);
        let promotion = stage.join("promotion/services");
        fs::create_dir_all(&promotion).unwrap();
        fs::write(
            promotion.join("orders.json"),
            bytes(services.get("orders").unwrap()).unwrap(),
        )
        .unwrap();
        fs::write(promotion.join(".tmp-interrupted"), b"partial service write").unwrap();
        promote_services(&repo, &services, &identity).unwrap();
        assert_eq!(repo.inputs().unwrap().services, services);
        assert!(directory_empty(&promotion).unwrap());
    }

    #[test]
    fn stage_work_symlink_is_rejected_without_touching_its_target() {
        let temporary = tempfile::tempdir().unwrap();
        Repository::init(temporary.path(), "Import symlink").unwrap();
        let repo = Repository::open(temporary.path()).unwrap();
        let identity = identity();
        let stage = stage_path(&repo, &identity).unwrap();
        install_owned_stage_root(&repo, &stage, &identity).unwrap();

        let outside = tempfile::tempdir().unwrap();
        let sentinel = outside.path().join("keep.txt");
        fs::write(&sentinel, b"untouched").unwrap();
        symlink(outside.path(), stage.join("work")).unwrap();
        assert!(open_or_create_stage(&repo, &stage, &identity).is_err());
        assert_eq!(fs::read(&sentinel).unwrap(), b"untouched");
    }

    #[test]
    fn completed_stage_cleanup_resumes_after_partial_directory_removal() {
        let temporary = tempfile::tempdir().unwrap();
        Repository::init(temporary.path(), "Import cleanup").unwrap();
        let repo = Repository::open(temporary.path()).unwrap();
        let identity = identity();
        let stage = stage_path(&repo, &identity).unwrap();
        let cleanup = stage.parent().unwrap().join(format!(
            ".snapshot-import-{}.cleanup",
            stage_token(&identity).unwrap()
        ));
        let cleanup_marker_path = repo
            .path(&format!(
                ".codeclew/cache/.snapshot-import-{}.cleanup.json",
                stage_token(&identity).unwrap()
            ))
            .unwrap();
        fs::create_dir(&cleanup).unwrap();
        fs::write(cleanup.join("remaining-work"), b"partial removal").unwrap();
        fs::write(&cleanup_marker_path, cleanup_marker(&identity).unwrap()).unwrap();

        cleanup_stage(&repo, &stage, &identity).unwrap();
        assert!(!cleanup.exists());
        assert!(!cleanup_marker_path.exists());
    }

    #[test]
    fn fresh_import_refuses_unrelated_cache_files_without_removing_them() {
        let temporary = tempfile::tempdir().unwrap();
        Repository::init(temporary.path(), "Import conflict").unwrap();
        let repo = Repository::open(temporary.path()).unwrap();
        let unrelated = repo.path(".codeclew/cache/other-capture.json").unwrap();
        fs::write(&unrelated, b"unowned").unwrap();

        let error = validate_cache_inventory(&repo, &BTreeSet::new(), None, false).unwrap_err();
        assert_eq!(error.code, ErrorCode::WwConflict);
        assert_eq!(fs::read(unrelated).unwrap(), b"unowned");
    }

    #[test]
    fn fresh_import_refuses_private_authority_namespaces() {
        for relative in [
            ".codeclew/accounts/account.json",
            ".codeclew/execution/accounts/account.json",
            ".codeclew/evidence/selected/evidence.json",
            ".codeclew/job-inputs/invocation.json",
            ".codeclew/job-results/invocation.json",
            ".codeclew/jobs/run.json",
            ".codeclew/proposals/proposal.json",
            ".codeclew/work/item/latest-run.json",
            ".codeclew/cache/pins/snapshot.json",
        ] {
            let temporary = tempfile::tempdir().unwrap();
            Repository::init(temporary.path(), "Private import conflict").unwrap();
            let repo = Repository::open(temporary.path()).unwrap();
            let marker = repo.path(relative).unwrap();
            fs::create_dir_all(marker.parent().unwrap()).unwrap();
            fs::write(&marker, b"retained private state").unwrap();

            let error = validate_empty_destination(&repo).unwrap_err();
            assert_eq!(error.code, ErrorCode::WwConflict, "{relative}");
            assert_eq!(fs::read(marker).unwrap(), b"retained private state");
        }
    }

    #[test]
    fn fresh_import_refuses_orphan_immutable_objects() {
        let temporary = tempfile::tempdir().unwrap();
        Repository::init(temporary.path(), "Orphan object conflict").unwrap();
        let repo = Repository::open(temporary.path()).unwrap();
        let reference = cache::put(&repo, "fixture/orphan-object", b"retained object").unwrap();

        let error = validate_empty_destination(&repo).unwrap_err();
        assert_eq!(error.code, ErrorCode::WwConflict);
        assert_eq!(
            cache::get(&repo, &reference, 1024).unwrap(),
            Some(b"retained object".to_vec())
        );
    }
}
