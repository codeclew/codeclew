use crate::canonical;
use crate::error::{ClewError, ErrorCode};
use crate::state::{ManagedDirectory, ManagedEntryKind, StateAuthority};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::ffi::OsStr;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::os::unix::fs::MetadataExt;
use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock, RwLock, Weak};

#[cfg(unix)]
use std::os::fd::AsRawFd;

const CAS_DOMAIN: &[u8] = b"codeclew-cas/v2\0";
pub const CAS_OBJECT_SCHEMA: &str = "codeclew-cas-object/2.0";
const CAS_PACK_SCHEMA: &str = "codeclew-cas-pack/3.0";
const PACK_VERIFICATION_SCHEMA: &str = "codeclew-cas-pack-verification/1.0";
const CATALOG_HEAD_SCHEMA: &str = "codeclew-cas-catalog-head/1.0";
const CATALOG_SNAPSHOT_SCHEMA: &str = "codeclew-cas-catalog-snapshot/1.0";
const CATALOG_RECORD_SCHEMA: &str = "codeclew-cas-catalog-record/1.0";
const MAX_PACK_INDEX_BYTES: usize = 64 * 1024 * 1024;
const MAX_PACK_VERIFICATION_BYTES: usize = 4096;
const MAX_CATALOG_HEAD_BYTES: usize = 4096;
const MAX_CATALOG_SNAPSHOT_BYTES: usize = 256 * 1024 * 1024;
const MAX_CATALOG_RECORD_BYTES: usize = MAX_PACK_INDEX_BYTES + 4096;
const CATALOG_SNAPSHOT_INTERVAL: u64 = 64;
const CATALOG_SNAPSHOT_TAIL_BYTES: u64 = 8 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PackManifest {
    schema: String,
    data_sha256: String,
    data_size: u64,
    objects: Vec<PackEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PackEntry {
    object: CasObject,
    offset: u64,
}

#[derive(Debug, Clone)]
struct PackLocation {
    data_name: String,
    entry: PackEntry,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CatalogPack {
    data_name: String,
    manifest: PackManifest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CatalogSnapshot {
    schema: String,
    sequence: u64,
    last_record_digest: Option<String>,
    packs: Vec<CatalogPack>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CatalogHead {
    schema: String,
    snapshot_name: String,
    snapshot_digest: String,
    snapshot_sequence: u64,
    last_record_digest: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
enum CatalogOperation {
    Add,
    Remove,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CatalogRecord {
    schema: String,
    sequence: u64,
    previous_record_digest: Option<String>,
    operation: CatalogOperation,
    pack: CatalogPack,
}

#[derive(Debug, Default)]
struct CatalogState {
    initialized: bool,
    sequence: u64,
    snapshot_sequence: u64,
    last_record_digest: Option<String>,
    snapshot_digest: Option<String>,
    tail_bytes: u64,
    packs: BTreeMap<String, PackManifest>,
    locations: BTreeMap<String, PackLocation>,
}

type SharedCatalog = Arc<RwLock<CatalogState>>;
type CatalogIdentity = (u64, u64);

fn catalog_registry() -> &'static Mutex<BTreeMap<CatalogIdentity, Weak<RwLock<CatalogState>>>> {
    static REGISTRY: OnceLock<Mutex<BTreeMap<CatalogIdentity, Weak<RwLock<CatalogState>>>>> =
        OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(BTreeMap::new()))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PackVerificationReceipt {
    schema: String,
    manifest_digest: String,
    data_sha256: String,
    data_size: u64,
    device: u64,
    inode: u64,
    modified_seconds: i64,
    modified_nanoseconds: i64,
    changed_seconds: i64,
    changed_nanoseconds: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CasObject {
    pub schema: String,
    pub object_schema: String,
    pub digest: String,
    pub size: u64,
}

pub const STORAGE_REPORT_SCHEMA: &str = "codeclew-storage-report/1.0";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum StorageAction {
    DryRun,
    Applied,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StorageReport {
    pub schema: String,
    pub action: StorageAction,
    pub root_files_scanned: u64,
    pub root_bytes_scanned: u64,
    pub reachable_objects: u64,
    pub packed_objects: u64,
    pub loose_objects: u64,
    pub pack_bytes: u64,
    pub loose_bytes: u64,
    pub catalog_metadata_bytes: u64,
    pub reclaimable_packs: u64,
    pub reclaimable_loose_objects: u64,
    pub reclaimable_orphan_pack_files: u64,
    pub reclaimable_bytes: u64,
    pub reclaimed_bytes: u64,
    pub retained_mixed_packs: u64,
}

#[derive(Debug)]
struct StoragePlan {
    report: StorageReport,
    dead_packs: Vec<CatalogPack>,
    dead_loose: Vec<String>,
    orphan_pack_files: Vec<std::ffi::OsString>,
}

impl CasObject {
    /// Derive the exact immutable reference that `CasStore::put` will publish.
    /// This is useful when a higher-level content authority must bind the CAS
    /// identity before the object is durably written.
    pub fn for_bytes(object_schema: &str, bytes: &[u8]) -> Result<Self, ClewError> {
        validate_object_schema(object_schema)?;
        Ok(Self {
            schema: CAS_OBJECT_SCHEMA.into(),
            object_schema: object_schema.into(),
            digest: object_digest(object_schema, bytes),
            size: bytes.len() as u64,
        })
    }
}

#[derive(Debug)]
pub struct CasLease {
    object: CasObject,
    bytes: Vec<u8>,
    #[allow(dead_code)]
    lock: File,
}

impl CasLease {
    pub fn object(&self) -> &CasObject {
        &self.object
    }

    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
}

#[derive(Debug, Clone)]
pub struct CasStore {
    objects: ManagedDirectory,
    packs: ManagedDirectory,
    catalog: ManagedDirectory,
    catalog_snapshots: ManagedDirectory,
    catalog_records: ManagedDirectory,
    locks: ManagedDirectory,
    quarantine: ManagedDirectory,
    _world_lease: Arc<File>,
    pack_catalog: SharedCatalog,
    #[cfg(test)]
    full_pack_verifications: Arc<std::sync::atomic::AtomicUsize>,
    #[cfg(test)]
    catalog_bootstrap_scans: Arc<std::sync::atomic::AtomicUsize>,
}

impl CasStore {
    pub fn open(authority: &StateAuthority) -> Result<Self, ClewError> {
        Self::open_with_catalog(authority, true)
    }

    fn open_with_catalog(
        authority: &StateAuthority,
        share_in_process: bool,
    ) -> Result<Self, ClewError> {
        let locks = authority.directory(Path::new("locks"))?;
        let world_lease = Arc::new(acquire_world_lease(&locks, LockMode::Shared)?);
        Self::open_with_catalog_and_lease(authority, share_in_process, locks, world_lease)
    }

    fn open_with_catalog_and_lease(
        authority: &StateAuthority,
        share_in_process: bool,
        locks: ManagedDirectory,
        world_lease: Arc<File>,
    ) -> Result<Self, ClewError> {
        let catalog = authority.directory(Path::new("objects/catalog-v1"))?;
        let identity = catalog.identity()?;
        let pack_catalog = if share_in_process {
            let mut registry = catalog_registry()
                .lock()
                .map_err(|_| internal("CAS catalog registry lock is poisoned"))?;
            registry.retain(|_, catalog| catalog.strong_count() > 0);
            if let Some(existing) = registry.get(&identity).and_then(Weak::upgrade) {
                existing
            } else {
                let created = Arc::new(RwLock::new(CatalogState::default()));
                registry.insert(identity, Arc::downgrade(&created));
                created
            }
        } else {
            Arc::new(RwLock::new(CatalogState::default()))
        };
        let store = Self {
            objects: authority.directory(Path::new("objects/sha256"))?,
            packs: authority.directory(Path::new("objects/packs-v3"))?,
            catalog,
            catalog_snapshots: authority.directory(Path::new("objects/catalog-v1/snapshots"))?,
            catalog_records: authority.directory(Path::new("objects/catalog-v1/records"))?,
            locks,
            quarantine: authority.directory(Path::new("quarantine"))?,
            _world_lease: world_lease,
            pack_catalog,
            #[cfg(test)]
            full_pack_verifications: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            #[cfg(test)]
            catalog_bootstrap_scans: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
        };
        store.ensure_catalog_initialized()?;
        Ok(store)
    }

    pub fn put(&self, object_schema: &str, bytes: &[u8]) -> Result<CasObject, ClewError> {
        let object = CasObject::for_bytes(object_schema, bytes)?;
        let digest = object.digest.clone();
        let lock = self.lock(&digest, LockMode::Exclusive)?;
        let (directory, name) = self.object_location(&digest)?;
        if directory.file_exists(OsStr::new(&name))? {
            match self.read_path(&object, &directory, OsStr::new(&name), bytes.len()) {
                Ok(existing) if existing == bytes => return Ok(object),
                Ok(_) | Err(_) => self.quarantine_locked(&directory, OsStr::new(&name), &digest)?,
            }
        }
        directory.atomic_write(OsStr::new(&name), bytes)?;
        let persisted = self.read_path(&object, &directory, OsStr::new(&name), bytes.len())?;
        if persisted != bytes {
            return Err(corrupt("CAS object changed during atomic publication"));
        }
        drop(lock);
        Ok(object)
    }

    /// Publish an indexed batch as one durable pack while preserving the
    /// caller's deterministic input order. Packed objects remain pack-first;
    /// publishing thousands of derived loose files would turn one sequential
    /// fsync into thousands of metadata transactions.
    pub fn put_batch(&self, objects: Vec<(String, Vec<u8>)>) -> Result<Vec<CasObject>, ClewError> {
        for (schema, _) in &objects {
            validate_object_schema(schema)?;
        }
        let prepared = objects
            .into_par_iter()
            .map(|(schema, bytes)| {
                let object = CasObject {
                    schema: CAS_OBJECT_SCHEMA.into(),
                    object_schema: schema,
                    digest: String::new(),
                    size: bytes.len() as u64,
                };
                let mut object = object;
                object.digest = object_digest(&object.object_schema, &bytes);
                (object, bytes)
            })
            .collect::<Vec<_>>();
        let references = prepared
            .iter()
            .map(|(object, _)| object.clone())
            .collect::<Vec<_>>();
        if prepared.is_empty() {
            return Ok(references);
        }
        let batch_lock = self.batch_lock()?;
        self.sync_catalog_locked()?;
        let mut missing = Vec::new();
        let mut missing_digests = BTreeSet::new();
        for (object, bytes) in &prepared {
            if self
                .read_from_catalog(object, bytes.len())?
                .is_some_and(|existing| existing == *bytes)
            {
                continue;
            }
            if missing_digests.insert(object.digest.clone()) {
                missing.push((object.clone(), bytes.clone()));
            }
        }
        if !missing.is_empty() {
            self.write_pack(&missing)?;
            for (object, bytes) in &missing {
                if self.read_from_catalog(object, bytes.len())?.as_deref() != Some(bytes.as_slice())
                {
                    return Err(corrupt("CAS pack verification changed object bytes"));
                }
            }
        }
        drop(batch_lock);
        Ok(references)
    }

    pub fn read(&self, object: &CasObject, max_bytes: usize) -> Result<CasLease, ClewError> {
        validate_reference(object)?;
        if object.size > max_bytes as u64 {
            return Err(ClewError::new(
                ErrorCode::ResourceLimit,
                "CAS object exceeds the caller's read budget",
            ));
        }
        let lock = self.lock(&object.digest, LockMode::Shared)?;
        if let Some(bytes) = self.read_from_catalog(object, max_bytes)? {
            return Ok(CasLease {
                object: object.clone(),
                bytes,
                lock,
            });
        }
        let (directory, name) = self.object_location(&object.digest)?;
        match self.read_path(object, &directory, OsStr::new(&name), max_bytes) {
            Ok(bytes) => Ok(CasLease {
                object: object.clone(),
                bytes,
                lock,
            }),
            Err(error) => {
                drop(lock);
                let exclusive = self.lock(&object.digest, LockMode::Exclusive)?;
                if directory.file_exists(OsStr::new(&name))? {
                    self.quarantine_locked(&directory, OsStr::new(&name), &object.digest)?;
                }
                self.sync_catalog()?;
                if let Some(bytes) = self.read_from_catalog(object, max_bytes)? {
                    drop(exclusive);
                    let lock = self.lock(&object.digest, LockMode::Shared)?;
                    return Ok(CasLease {
                        object: object.clone(),
                        bytes,
                        lock,
                    });
                }
                drop(exclusive);
                Err(error)
            }
        }
    }

    fn write_pack(&self, objects: &[(CasObject, Vec<u8>)]) -> Result<(), ClewError> {
        let temporary = format!(".pack-{}", uuid::Uuid::new_v4());
        let mut file = self.packs.create_file(OsStr::new(&temporary))?;
        let mut digest = Sha256::new();
        let mut offset = 0u64;
        let mut entries = Vec::with_capacity(objects.len());
        for (object, bytes) in objects {
            file.write_all(bytes).map_err(io_error)?;
            digest.update(bytes);
            entries.push(PackEntry {
                object: object.clone(),
                offset,
            });
            offset = offset
                .checked_add(object.size)
                .ok_or_else(|| corrupt("CAS pack size overflow"))?;
        }
        file.sync_all().map_err(io_error)?;
        drop(file);
        let data_sha256 = format!("sha256:{}", hex::encode(digest.finalize()));
        let manifest = PackManifest {
            schema: CAS_PACK_SCHEMA.into(),
            data_sha256,
            data_size: offset,
            objects: entries,
        };
        let bytes = canonical::bytes(&manifest).map_err(internal)?;
        if bytes.len() > MAX_PACK_INDEX_BYTES {
            let _ = self.packs.remove_file(OsStr::new(&temporary));
            return Err(ClewError::new(
                ErrorCode::ResourceLimit,
                "CAS pack index exceeds its bounded size",
            ));
        }
        let manifest_digest = canonical::hash(&manifest).map_err(internal)?;
        let component = digest_component(&manifest_digest)?;
        let data_name = format!("{component}.pack");
        let index_name = format!("{component}.json");
        let data_exists = self.packs.file_exists(OsStr::new(&data_name))?;
        let index_exists = self.packs.file_exists(OsStr::new(&index_name))?;
        match (data_exists, index_exists) {
            (true, true) => {
                let _ = self.packs.remove_file(OsStr::new(&temporary));
            }
            (true, false) => {
                let _ = self.packs.remove_file(OsStr::new(&temporary));
                self.verify_pack_data(&data_name, component, &manifest_digest, &manifest)?;
                self.packs.atomic_write(OsStr::new(&index_name), &bytes)?;
            }
            (false, true) => {
                let existing = self
                    .packs
                    .read_file(OsStr::new(&index_name), MAX_PACK_INDEX_BYTES)
                    .map_err(|_| corrupt("CAS orphan pack index is unsafe"))?;
                if existing != bytes {
                    let _ = self.packs.remove_file(OsStr::new(&temporary));
                    return Err(corrupt("CAS orphan pack index has another authority"));
                }
                self.packs.rename_to(
                    OsStr::new(&temporary),
                    &self.packs,
                    OsStr::new(&data_name),
                )?;
            }
            (false, false) => {
                self.packs.rename_to(
                    OsStr::new(&temporary),
                    &self.packs,
                    OsStr::new(&data_name),
                )?;
                self.packs.atomic_write(OsStr::new(&index_name), &bytes)?;
            }
        }
        self.verify_pack_pair(&data_name, &index_name, Some(&manifest))?;
        self.append_catalog_record_locked(CatalogOperation::Add, &data_name, &manifest)?;
        self.prune_pack_metadata(&data_name)?;
        Ok(())
    }

    fn read_from_catalog(
        &self,
        object: &CasObject,
        max_bytes: usize,
    ) -> Result<Option<Vec<u8>>, ClewError> {
        let location = self
            .pack_catalog
            .read()
            .map_err(|_| internal("CAS pack catalog lock is poisoned"))?
            .locations
            .get(&object.digest)
            .cloned();
        let Some(location) = location else {
            return Ok(None);
        };
        if location.entry.object != *object || object.size > max_bytes as u64 {
            return Err(corrupt("CAS pack reference differs from requested object"));
        }
        let mut file = self
            .packs
            .open_file(OsStr::new(&location.data_name))
            .map_err(|_| corrupt("CAS packed data is missing or unsafe"))?;
        file.seek(SeekFrom::Start(location.entry.offset))
            .map_err(|_| corrupt("CAS packed object offset is unreadable"))?;
        let mut bytes = vec![0; object.size as usize];
        file.read_exact(&mut bytes)
            .map_err(|_| corrupt("CAS packed object is truncated"))?;
        if object_digest(&object.object_schema, &bytes) != object.digest {
            return Err(corrupt("CAS packed object digest mismatch"));
        }
        Ok(Some(bytes))
    }

    fn ensure_catalog_initialized(&self) -> Result<(), ClewError> {
        if self
            .pack_catalog
            .read()
            .map_err(|_| internal("CAS pack catalog lock is poisoned"))?
            .initialized
        {
            return Ok(());
        }
        let batch_lock = self.batch_lock()?;
        if !self
            .pack_catalog
            .read()
            .map_err(|_| internal("CAS pack catalog lock is poisoned"))?
            .initialized
        {
            self.sync_catalog_locked()?;
        }
        drop(batch_lock);
        Ok(())
    }

    fn sync_catalog(&self) -> Result<(), ClewError> {
        let batch_lock = self.batch_lock()?;
        self.sync_catalog_locked()?;
        drop(batch_lock);
        Ok(())
    }

    /// Synchronize only at process initialization, before publication, or on
    /// a digest miss. Pack bytes are immutable, so a known catalog location
    /// never needs a global rescan merely to be read again.
    fn sync_catalog_locked(&self) -> Result<(), ClewError> {
        if !self.catalog.file_exists(OsStr::new("head.json"))? {
            if self
                .catalog_snapshots
                .entries()?
                .iter()
                .any(|name| Path::new(name).extension() == Some(OsStr::new("snapshot")))
                || self
                    .catalog_records
                    .entries()?
                    .iter()
                    .any(|name| Path::new(name).extension() == Some(OsStr::new("record")))
            {
                return Err(corrupt(
                    "CAS catalog head is missing from non-empty metadata",
                ));
            }
            let snapshot = self.bootstrap_catalog_snapshot()?;
            self.publish_catalog_snapshot_locked(&snapshot)?;
        }

        let head_bytes = self
            .catalog
            .read_file(OsStr::new("head.json"), MAX_CATALOG_HEAD_BYTES)
            .map_err(|_| corrupt("CAS catalog head is missing or unsafe"))?;
        let head: CatalogHead = serde_json::from_slice(&head_bytes)
            .map_err(|_| corrupt("CAS catalog head is invalid"))?;
        if head.schema != CATALOG_HEAD_SCHEMA
            || canonical::bytes(&head).map_err(internal)? != head_bytes
            || digest_component(&head.snapshot_digest).is_err()
            || Path::new(&head.snapshot_name).extension() != Some(OsStr::new("snapshot"))
            || head.snapshot_name
                != format!("{}.snapshot", digest_component(&head.snapshot_digest)?)
        {
            return Err(corrupt("CAS catalog head authority mismatch"));
        }

        let needs_snapshot = {
            let state = self
                .pack_catalog
                .read()
                .map_err(|_| internal("CAS pack catalog lock is poisoned"))?;
            if state.initialized
                && head.snapshot_sequence == state.snapshot_sequence
                && state.snapshot_digest.as_deref() != Some(head.snapshot_digest.as_str())
            {
                return Err(corrupt("CAS catalog head changed snapshot authority"));
            }
            !state.initialized || head.snapshot_sequence > state.sequence
        };
        if needs_snapshot {
            let bytes = self
                .catalog_snapshots
                .read_file(OsStr::new(&head.snapshot_name), MAX_CATALOG_SNAPSHOT_BYTES)
                .map_err(|_| corrupt("CAS catalog snapshot is missing or unsafe"))?;
            if canonical::hash_bytes(&bytes) != head.snapshot_digest {
                return Err(corrupt("CAS catalog snapshot digest mismatch"));
            }
            let snapshot: CatalogSnapshot = serde_json::from_slice(&bytes)
                .map_err(|_| corrupt("CAS catalog snapshot is invalid"))?;
            if snapshot.schema != CATALOG_SNAPSHOT_SCHEMA
                || canonical::bytes(&snapshot).map_err(internal)? != bytes
                || snapshot.sequence != head.snapshot_sequence
                || snapshot.last_record_digest != head.last_record_digest
            {
                return Err(corrupt("CAS catalog snapshot authority mismatch"));
            }
            let rebuilt = catalog_state_from_snapshot(&snapshot, &head.snapshot_digest)?;
            *self
                .pack_catalog
                .write()
                .map_err(|_| internal("CAS pack catalog lock is poisoned"))? = rebuilt;
        }

        let mut record_names = self
            .catalog_records
            .entries()?
            .into_iter()
            .filter(|name| Path::new(name).extension() == Some(OsStr::new("record")))
            .collect::<Vec<_>>();
        record_names.sort();
        for name in record_names {
            let name_text = name
                .to_str()
                .ok_or_else(|| corrupt("CAS catalog record name is not UTF-8"))?;
            let bytes = self
                .catalog_records
                .read_file(&name, MAX_CATALOG_RECORD_BYTES)
                .map_err(|_| corrupt("CAS catalog record is missing or unsafe"))?;
            let record: CatalogRecord = serde_json::from_slice(&bytes)
                .map_err(|_| corrupt("CAS catalog record is invalid"))?;
            let record_digest = canonical::hash(&record).map_err(internal)?;
            if record.schema != CATALOG_RECORD_SCHEMA
                || canonical::bytes(&record).map_err(internal)? != bytes
                || catalog_record_name(record.sequence, &record_digest)? != name_text
            {
                return Err(corrupt("CAS catalog record authority mismatch"));
            }
            let mut state = self
                .pack_catalog
                .write()
                .map_err(|_| internal("CAS pack catalog lock is poisoned"))?;
            if record.sequence <= state.sequence {
                continue;
            }
            if record.sequence != state.sequence + 1
                || record.previous_record_digest != state.last_record_digest
            {
                return Err(corrupt("CAS catalog record chain is discontinuous"));
            }
            apply_catalog_record(&mut state, &record)?;
            state.sequence = record.sequence;
            state.last_record_digest = Some(record_digest);
            state.tail_bytes = state.tail_bytes.saturating_add(bytes.len() as u64);
        }
        self.prune_catalog_metadata_locked(&head.snapshot_name, head.snapshot_sequence)?;
        self.prune_redundant_pack_metadata_locked()
    }

    fn bootstrap_catalog_snapshot(&self) -> Result<CatalogSnapshot, ClewError> {
        #[cfg(test)]
        self.catalog_bootstrap_scans
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let indexes = self
            .packs
            .entries()?
            .into_iter()
            .filter(|name| Path::new(name).extension() == Some(OsStr::new("json")))
            .collect::<Vec<_>>();
        let mut verified = indexes
            .par_iter()
            .map(|index_name| {
                let component = Path::new(index_name)
                    .file_stem()
                    .and_then(OsStr::to_str)
                    .ok_or_else(|| corrupt("CAS pack index name is invalid"))?;
                let data_name = format!("{component}.pack");
                let index_name = index_name
                    .to_str()
                    .ok_or_else(|| corrupt("CAS pack index name is not UTF-8"))?;
                let manifest = self.verify_pack_pair(&data_name, index_name, None)?;
                Ok((data_name, manifest))
            })
            .collect::<Result<Vec<_>, ClewError>>()?;
        verified.sort_by(|left, right| left.0.cmp(&right.0));
        Ok(CatalogSnapshot {
            schema: CATALOG_SNAPSHOT_SCHEMA.into(),
            sequence: 0,
            last_record_digest: None,
            packs: verified
                .into_iter()
                .map(|(data_name, manifest)| CatalogPack {
                    data_name,
                    manifest,
                })
                .collect(),
        })
    }

    fn append_catalog_record_locked(
        &self,
        operation: CatalogOperation,
        data_name: &str,
        manifest: &PackManifest,
    ) -> Result<(), ClewError> {
        let state = self
            .pack_catalog
            .read()
            .map_err(|_| internal("CAS pack catalog lock is poisoned"))?;
        let record = CatalogRecord {
            schema: CATALOG_RECORD_SCHEMA.into(),
            sequence: state
                .sequence
                .checked_add(1)
                .ok_or_else(|| corrupt("CAS catalog sequence overflow"))?,
            previous_record_digest: state.last_record_digest.clone(),
            operation,
            pack: CatalogPack {
                data_name: data_name.into(),
                manifest: manifest.clone(),
            },
        };
        drop(state);
        let bytes = canonical::bytes(&record).map_err(internal)?;
        if bytes.len() > MAX_CATALOG_RECORD_BYTES {
            return Err(ClewError::new(
                ErrorCode::ResourceLimit,
                "CAS catalog record exceeds its bounded size",
            ));
        }
        let digest = canonical::hash(&record).map_err(internal)?;
        let name = catalog_record_name(record.sequence, &digest)?;
        if !self
            .catalog_records
            .atomic_create(OsStr::new(&name), &bytes)?
        {
            let existing = self
                .catalog_records
                .read_file(OsStr::new(&name), MAX_CATALOG_RECORD_BYTES)?;
            if existing != bytes {
                return Err(corrupt("CAS catalog sequence has another authority"));
            }
        }
        {
            let mut state = self
                .pack_catalog
                .write()
                .map_err(|_| internal("CAS pack catalog lock is poisoned"))?;
            apply_catalog_record(&mut state, &record)?;
            state.sequence = record.sequence;
            state.last_record_digest = Some(digest);
            state.tail_bytes = state.tail_bytes.saturating_add(bytes.len() as u64);
        }
        self.maybe_snapshot_catalog_locked()
    }

    fn maybe_snapshot_catalog_locked(&self) -> Result<(), ClewError> {
        let snapshot = {
            let state = self
                .pack_catalog
                .read()
                .map_err(|_| internal("CAS pack catalog lock is poisoned"))?;
            if !catalog_snapshot_due(&state) {
                return Ok(());
            }
            snapshot_from_catalog_state(&state)
        };
        self.publish_catalog_snapshot_locked(&snapshot)
    }

    fn publish_catalog_snapshot_locked(&self, snapshot: &CatalogSnapshot) -> Result<(), ClewError> {
        let bytes = canonical::bytes(snapshot).map_err(internal)?;
        if bytes.len() > MAX_CATALOG_SNAPSHOT_BYTES {
            return Err(ClewError::new(
                ErrorCode::ResourceLimit,
                "CAS catalog snapshot exceeds its bounded size",
            ));
        }
        let digest = canonical::hash(snapshot).map_err(internal)?;
        let snapshot_name = format!("{}.snapshot", digest_component(&digest)?);
        if !self
            .catalog_snapshots
            .atomic_create(OsStr::new(&snapshot_name), &bytes)?
        {
            let existing = self
                .catalog_snapshots
                .read_file(OsStr::new(&snapshot_name), MAX_CATALOG_SNAPSHOT_BYTES)?;
            if existing != bytes {
                return Err(corrupt("CAS catalog snapshot has another authority"));
            }
        }
        let head = CatalogHead {
            schema: CATALOG_HEAD_SCHEMA.into(),
            snapshot_name: snapshot_name.clone(),
            snapshot_digest: digest.clone(),
            snapshot_sequence: snapshot.sequence,
            last_record_digest: snapshot.last_record_digest.clone(),
        };
        self.catalog.atomic_write(
            OsStr::new("head.json"),
            &canonical::bytes(&head).map_err(internal)?,
        )?;
        {
            let mut state = self
                .pack_catalog
                .write()
                .map_err(|_| internal("CAS pack catalog lock is poisoned"))?;
            if !state.initialized || snapshot.sequence >= state.sequence {
                *state = catalog_state_from_snapshot(snapshot, &digest)?;
            }
        }
        self.prune_catalog_metadata_locked(&snapshot_name, snapshot.sequence)?;
        self.prune_redundant_pack_metadata_locked()
    }

    fn prune_catalog_metadata_locked(
        &self,
        current_snapshot: &str,
        covered_sequence: u64,
    ) -> Result<(), ClewError> {
        for name in self.catalog_snapshots.entries()? {
            if name.to_string_lossy().starts_with(".tmp-")
                || (Path::new(&name).extension() == Some(OsStr::new("snapshot"))
                    && name != OsStr::new(current_snapshot))
            {
                self.catalog_snapshots.remove_file(&name)?;
            }
        }
        for name in self.catalog_records.entries()? {
            if name.to_string_lossy().starts_with(".tmp-") {
                self.catalog_records.remove_file(&name)?;
                continue;
            }
            if Path::new(&name).extension() != Some(OsStr::new("record")) {
                continue;
            }
            let Some(sequence) = catalog_record_sequence(&name) else {
                return Err(corrupt("CAS catalog record name is invalid"));
            };
            if sequence <= covered_sequence {
                self.catalog_records.remove_file(&name)?;
            }
        }
        Ok(())
    }

    fn prune_redundant_pack_metadata_locked(&self) -> Result<(), ClewError> {
        let data_names = self
            .pack_catalog
            .read()
            .map_err(|_| internal("CAS pack catalog lock is poisoned"))?
            .packs
            .keys()
            .cloned()
            .collect::<Vec<_>>();
        for data_name in data_names {
            self.prune_pack_metadata(&data_name)?;
        }
        Ok(())
    }

    fn prune_pack_metadata(&self, data_name: &str) -> Result<(), ClewError> {
        let component = Path::new(data_name)
            .file_stem()
            .and_then(OsStr::to_str)
            .ok_or_else(|| corrupt("CAS catalog pack name is invalid"))?;
        for name in [format!("{component}.json"), format!("{component}.verified")] {
            if self.packs.file_exists(OsStr::new(&name))? {
                self.packs.remove_file(OsStr::new(&name))?;
            }
        }
        Ok(())
    }

    fn verify_pack_pair(
        &self,
        data_name: &str,
        index_name: &str,
        expected: Option<&PackManifest>,
    ) -> Result<PackManifest, ClewError> {
        let bytes = self
            .packs
            .read_file(OsStr::new(index_name), MAX_PACK_INDEX_BYTES)
            .map_err(|_| corrupt("CAS pack index is missing or unsafe"))?;
        let manifest: PackManifest =
            serde_json::from_slice(&bytes).map_err(|_| corrupt("CAS pack index is invalid"))?;
        if canonical::bytes(&manifest).map_err(internal)? != bytes
            || expected.is_some_and(|expected| expected != &manifest)
        {
            return Err(corrupt("CAS pack authority mismatch"));
        }
        let manifest_digest = canonical::hash(&manifest).map_err(internal)?;
        let component = digest_component(&manifest_digest)?;
        validate_pack_manifest(data_name, &manifest)?;
        self.verify_pack_data(data_name, component, &manifest_digest, &manifest)?;
        Ok(manifest)
    }

    fn verify_pack_data(
        &self,
        data_name: &str,
        component: &str,
        manifest_digest: &str,
        manifest: &PackManifest,
    ) -> Result<(), ClewError> {
        let mut data_file = self
            .packs
            .open_file(OsStr::new(data_name))
            .map_err(|_| corrupt("CAS pack data is missing or unsafe"))?;
        let data_metadata = data_file.metadata().map_err(io_error)?;
        if data_metadata.len() != manifest.data_size {
            return Err(corrupt("CAS pack data size differs from its manifest"));
        }
        let receipt = pack_verification_receipt(manifest_digest, manifest, &data_metadata);
        let receipt_name = format!("{component}.verified");
        if self.packs.file_exists(OsStr::new(&receipt_name))?
            && let Ok(bytes) = self
                .packs
                .read_file(OsStr::new(&receipt_name), MAX_PACK_VERIFICATION_BYTES)
            && let Ok(cached) = serde_json::from_slice::<PackVerificationReceipt>(&bytes)
            && canonical::bytes(&cached).ok().as_deref() == Some(bytes.as_slice())
            && cached == receipt
        {
            return Ok(());
        }
        #[cfg(test)]
        self.full_pack_verifications
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let mut digest = Sha256::new();
        let mut buffer = vec![0u8; 1024 * 1024];
        loop {
            let read = data_file.read(&mut buffer).map_err(io_error)?;
            if read == 0 {
                break;
            }
            digest.update(&buffer[..read]);
        }
        let actual = format!("sha256:{}", hex::encode(digest.finalize()));
        if actual != manifest.data_sha256 {
            return Err(corrupt("CAS pack data digest differs from its manifest"));
        }
        if pack_verification_receipt(
            manifest_digest,
            manifest,
            &data_file.metadata().map_err(io_error)?,
        ) != receipt
        {
            return Err(corrupt("CAS pack data changed during verification"));
        }
        self.packs.atomic_write(
            OsStr::new(&receipt_name),
            &canonical::bytes(&receipt).map_err(internal)?,
        )?;
        Ok(())
    }

    fn read_path(
        &self,
        object: &CasObject,
        directory: &ManagedDirectory,
        name: &OsStr,
        max_bytes: usize,
    ) -> Result<Vec<u8>, ClewError> {
        let file = directory
            .open_file(name)
            .map_err(|_| corrupt("CAS object is missing or unsafe"))?;
        let metadata = file.metadata().map_err(io_error)?;
        if metadata.len() != object.size || metadata.len() > max_bytes as u64 {
            return Err(corrupt("CAS object metadata is invalid"));
        }
        let mut bytes = Vec::with_capacity(metadata.len() as usize);
        file.take(max_bytes as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(io_error)?;
        if bytes.len() != object.size as usize
            || object_digest(&object.object_schema, &bytes) != object.digest
        {
            return Err(corrupt("CAS object digest mismatch"));
        }
        Ok(bytes)
    }

    fn quarantine_locked(
        &self,
        directory: &ManagedDirectory,
        name: &OsStr,
        digest: &str,
    ) -> Result<(), ClewError> {
        let destination = format!(
            "cas-{}-{}",
            digest
                .strip_prefix("sha256:")
                .ok_or_else(|| corrupt("CAS digest prefix is invalid"))?,
            uuid::Uuid::new_v4()
        );
        directory.rename_to(name, &self.quarantine, OsStr::new(&destination))
    }

    fn object_location(&self, digest: &str) -> Result<(ManagedDirectory, String), ClewError> {
        let hex = digest_component(digest)?;
        Ok((
            self.objects.child(Path::new(&hex[..2]))?,
            hex[2..].to_owned(),
        ))
    }

    fn lock(&self, digest: &str, mode: LockMode) -> Result<File, ClewError> {
        let component = digest_component(digest)?;
        let name = format!("cas-{component}.lock");
        let file = self.locks.open_lock(OsStr::new(&name))?;
        #[cfg(unix)]
        {
            let operation = match mode {
                LockMode::Shared => libc::LOCK_SH,
                LockMode::Exclusive => libc::LOCK_EX,
            };
            if unsafe { libc::flock(file.as_raw_fd(), operation) } != 0 {
                return Err(io_error(std::io::Error::last_os_error()));
            }
        }
        Ok(file)
    }

    fn batch_lock(&self) -> Result<File, ClewError> {
        let file = self.locks.open_lock(OsStr::new("cas-pack-batch.lock"))?;
        #[cfg(unix)]
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) } != 0 {
            return Err(io_error(std::io::Error::last_os_error()));
        }
        Ok(file)
    }

    #[cfg(test)]
    fn object_path(&self, digest: &str) -> Result<std::path::PathBuf, ClewError> {
        let (directory, name) = self.object_location(digest)?;
        Ok(directory.path().join(name))
    }

    fn storage_plan(&self, authority: &StateAuthority) -> Result<StoragePlan, ClewError> {
        let mut roots = BTreeMap::<String, CasObject>::new();
        let mut root_files_scanned = 0u64;
        let mut root_bytes_scanned = 0u64;
        for name in [
            "repos",
            "sessions",
            "missions",
            "workspaces",
            "threads",
            "runs",
            "generations",
        ] {
            scan_managed_roots(
                &authority.directory(Path::new(name))?,
                0,
                &mut root_files_scanned,
                &mut root_bytes_scanned,
                &mut roots,
            )?;
        }

        let mut reachable = BTreeSet::new();
        let mut pending = roots.values().cloned().collect::<VecDeque<_>>();
        let mut authorities = roots;
        while let Some(object) = pending.pop_front() {
            validate_reference(&object)?;
            if !reachable.insert(object.digest.clone()) {
                continue;
            }
            let limit = usize::try_from(object.size).map_err(|_| {
                ClewError::new(
                    ErrorCode::ResourceLimit,
                    "CAS reachability object exceeds host size",
                )
            })?;
            if limit > MAX_REACHABILITY_OBJECT_BYTES {
                return Err(ClewError::new(
                    ErrorCode::ResourceLimit,
                    "CAS reachability object exceeds the 512 MiB safety bound",
                ));
            }
            let lease = self.read(&object, limit)?;
            if let Ok(value) = serde_json::from_slice::<Value>(lease.bytes()) {
                collect_cas_references(&value, &mut authorities, &mut pending)?;
            }
        }

        let catalog = self
            .pack_catalog
            .read()
            .map_err(|_| internal("CAS pack catalog lock is poisoned"))?;
        let mut dead_packs = Vec::new();
        let mut packed_objects = 0u64;
        let mut pack_bytes = 0u64;
        let mut reclaimable_bytes = 0u64;
        let mut retained_mixed_packs = 0u64;
        let mut expected_pack_files = BTreeSet::new();
        for (data_name, manifest) in &catalog.packs {
            let component = Path::new(data_name)
                .file_stem()
                .and_then(OsStr::to_str)
                .ok_or_else(|| corrupt("CAS catalog pack name is invalid"))?;
            let index_name = format!("{component}.json");
            let receipt_name = format!("{component}.verified");
            expected_pack_files.insert(data_name.clone());
            expected_pack_files.insert(index_name.clone());
            expected_pack_files.insert(receipt_name.clone());
            let physical_bytes = self.packs.file_len(OsStr::new(data_name))?
                + optional_file_len(&self.packs, &index_name)?
                + optional_file_len(&self.packs, &receipt_name)?;
            pack_bytes = pack_bytes.saturating_add(physical_bytes);
            packed_objects = packed_objects.saturating_add(manifest.objects.len() as u64);
            let live = manifest
                .objects
                .iter()
                .filter(|entry| reachable.contains(&entry.object.digest))
                .count();
            if live == 0 {
                reclaimable_bytes = reclaimable_bytes.saturating_add(physical_bytes);
                dead_packs.push(CatalogPack {
                    data_name: data_name.clone(),
                    manifest: manifest.clone(),
                });
            } else if live < manifest.objects.len() {
                retained_mixed_packs += 1;
            }
        }
        let packed_digests = catalog.locations.keys().cloned().collect::<BTreeSet<_>>();
        drop(catalog);

        let mut orphan_pack_files = Vec::new();
        let mut orphan_pack_bytes = 0u64;
        for name in self.packs.entries()? {
            let text = name
                .to_str()
                .ok_or_else(|| corrupt("CAS pack directory entry is not UTF-8"))?;
            if expected_pack_files.contains(text) {
                continue;
            }
            if is_reclaimable_orphan_pack_file(text) {
                orphan_pack_bytes = orphan_pack_bytes.saturating_add(self.packs.file_len(&name)?);
                orphan_pack_files.push(name);
            }
        }
        reclaimable_bytes = reclaimable_bytes.saturating_add(orphan_pack_bytes);

        let mut loose_objects = 0u64;
        let mut loose_bytes = 0u64;
        let mut dead_loose = Vec::new();
        for prefix in self.objects.entries()? {
            let prefix_text = prefix
                .to_str()
                .filter(|value| is_lower_hex(value, 2))
                .ok_or_else(|| corrupt("CAS loose-object prefix is invalid"))?;
            if self.objects.entry_kind(&prefix)? != ManagedEntryKind::Directory {
                return Err(corrupt("CAS loose-object prefix is not a directory"));
            }
            let directory = self.objects.existing_child(&prefix)?;
            for name in directory.entries()? {
                let name_text = name
                    .to_str()
                    .filter(|value| is_lower_hex(value, 62))
                    .ok_or_else(|| corrupt("CAS loose-object name is invalid"))?;
                if directory.entry_kind(&name)? != ManagedEntryKind::File {
                    return Err(corrupt("CAS loose object is not a regular file"));
                }
                let digest = format!("sha256:{prefix_text}{name_text}");
                let bytes = directory.file_len(&name)?;
                loose_objects += 1;
                loose_bytes = loose_bytes.saturating_add(bytes);
                if !reachable.contains(&digest) || packed_digests.contains(&digest) {
                    reclaimable_bytes = reclaimable_bytes.saturating_add(bytes);
                    dead_loose.push(digest);
                }
            }
        }

        let catalog_metadata_bytes = catalog_metadata_bytes(self)?;
        Ok(StoragePlan {
            report: StorageReport {
                schema: STORAGE_REPORT_SCHEMA.into(),
                action: StorageAction::DryRun,
                root_files_scanned,
                root_bytes_scanned,
                reachable_objects: reachable.len() as u64,
                packed_objects,
                loose_objects,
                pack_bytes,
                loose_bytes,
                catalog_metadata_bytes,
                reclaimable_packs: dead_packs.len() as u64,
                reclaimable_loose_objects: dead_loose.len() as u64,
                reclaimable_orphan_pack_files: orphan_pack_files.len() as u64,
                reclaimable_bytes,
                reclaimed_bytes: 0,
                retained_mixed_packs,
            },
            dead_packs,
            dead_loose,
            orphan_pack_files,
        })
    }
}

const MAX_REACHABILITY_ROOT_FILE_BYTES: usize = 256 * 1024 * 1024;
const MAX_REACHABILITY_OBJECT_BYTES: usize = 512 * 1024 * 1024;
const MAX_REACHABILITY_ROOT_FILES: u64 = 1_000_000;
const MAX_REACHABILITY_ROOT_BYTES: u64 = 8 * 1024 * 1024 * 1024;
const MAX_REACHABILITY_DEPTH: usize = 64;

pub fn storage_status(authority: &StateAuthority) -> Result<StorageReport, ClewError> {
    Ok(CasStore::open(authority)?.storage_plan(authority)?.report)
}

pub fn garbage_collect_storage(authority: &StateAuthority) -> Result<StorageReport, ClewError> {
    let locks = authority.directory(Path::new("locks"))?;
    let world_lease = Arc::new(acquire_world_lease(&locks, LockMode::Exclusive)?);
    let store =
        CasStore::open_with_catalog_and_lease(authority, false, locks, Arc::clone(&world_lease))?;
    let mut plan = store.storage_plan(authority)?;
    let batch_lock = store.batch_lock()?;
    store.sync_catalog_locked()?;
    for pack in &plan.dead_packs {
        store.append_catalog_record_locked(
            CatalogOperation::Remove,
            &pack.data_name,
            &pack.manifest,
        )?;
        let component = Path::new(&pack.data_name)
            .file_stem()
            .and_then(OsStr::to_str)
            .ok_or_else(|| corrupt("CAS garbage-collection pack name is invalid"))?;
        for name in [
            pack.data_name.clone(),
            format!("{component}.json"),
            format!("{component}.verified"),
        ] {
            if store.packs.file_exists(OsStr::new(&name))? {
                store.packs.remove_file(OsStr::new(&name))?;
            }
        }
    }
    for digest in &plan.dead_loose {
        let (directory, name) = store.object_location(digest)?;
        if directory.file_exists(OsStr::new(&name))? {
            directory.remove_file(OsStr::new(&name))?;
        }
    }
    for name in &plan.orphan_pack_files {
        if store.packs.file_exists(name)? {
            store.packs.remove_file(name)?;
        }
    }
    drop(batch_lock);
    plan.report.action = StorageAction::Applied;
    plan.report.reclaimed_bytes = plan.report.reclaimable_bytes;
    drop(store);
    drop(world_lease);
    Ok(plan.report)
}

fn scan_managed_roots(
    directory: &ManagedDirectory,
    depth: usize,
    files_scanned: &mut u64,
    bytes_scanned: &mut u64,
    references: &mut BTreeMap<String, CasObject>,
) -> Result<(), ClewError> {
    if depth > MAX_REACHABILITY_DEPTH {
        return Err(ClewError::new(
            ErrorCode::ResourceLimit,
            "managed-state reachability depth exceeds its bound",
        ));
    }
    for name in directory.entries()? {
        match directory.entry_kind(&name)? {
            ManagedEntryKind::Directory => scan_managed_roots(
                &directory.existing_child(&name)?,
                depth + 1,
                files_scanned,
                bytes_scanned,
                references,
            )?,
            ManagedEntryKind::File => {
                let extension = Path::new(&name).extension();
                if extension != Some(OsStr::new("json")) && extension != Some(OsStr::new("jsonl")) {
                    continue;
                }
                *files_scanned = files_scanned.saturating_add(1);
                if *files_scanned > MAX_REACHABILITY_ROOT_FILES {
                    return Err(ClewError::new(
                        ErrorCode::ResourceLimit,
                        "managed-state reachability file count exceeds its bound",
                    ));
                }
                let bytes = directory
                    .read_file(&name, MAX_REACHABILITY_ROOT_FILE_BYTES)
                    .map_err(|_| corrupt("managed-state reachability root is unsafe"))?;
                *bytes_scanned = bytes_scanned.saturating_add(bytes.len() as u64);
                if *bytes_scanned > MAX_REACHABILITY_ROOT_BYTES {
                    return Err(ClewError::new(
                        ErrorCode::ResourceLimit,
                        "managed-state reachability bytes exceed the 8 GiB bound",
                    ));
                }
                if extension == Some(OsStr::new("jsonl")) {
                    for line in bytes.split(|byte| *byte == b'\n') {
                        if line.iter().all(u8::is_ascii_whitespace) {
                            continue;
                        }
                        let value: Value = serde_json::from_slice(line).map_err(|_| {
                            corrupt("managed-state JSONL reachability root is invalid")
                        })?;
                        collect_root_cas_references(&value, references)?;
                    }
                } else {
                    let value: Value = serde_json::from_slice(&bytes)
                        .map_err(|_| corrupt("managed-state JSON reachability root is invalid"))?;
                    collect_root_cas_references(&value, references)?;
                }
            }
        }
    }
    Ok(())
}

fn collect_root_cas_references(
    value: &Value,
    references: &mut BTreeMap<String, CasObject>,
) -> Result<(), ClewError> {
    let mut pending = VecDeque::new();
    collect_cas_references(value, references, &mut pending)
}

fn collect_cas_references(
    value: &Value,
    authorities: &mut BTreeMap<String, CasObject>,
    pending: &mut VecDeque<CasObject>,
) -> Result<(), ClewError> {
    match value {
        Value::Object(map) => {
            if map.get("schema").and_then(Value::as_str) == Some(CAS_OBJECT_SCHEMA) {
                let object: CasObject = serde_json::from_value(value.clone())
                    .map_err(|_| corrupt("embedded CAS reference is invalid"))?;
                validate_reference(&object)?;
                match authorities.get(&object.digest) {
                    Some(existing) if existing != &object => {
                        return Err(corrupt("embedded CAS digest has conflicting authority"));
                    }
                    Some(_) => {}
                    None => {
                        authorities.insert(object.digest.clone(), object.clone());
                        pending.push_back(object);
                    }
                }
                return Ok(());
            }
            for nested in map.values() {
                collect_cas_references(nested, authorities, pending)?;
            }
        }
        Value::Array(items) => {
            for nested in items {
                collect_cas_references(nested, authorities, pending)?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn optional_file_len(directory: &ManagedDirectory, name: &str) -> Result<u64, ClewError> {
    if directory.file_exists(OsStr::new(name))? {
        directory.file_len(OsStr::new(name))
    } else {
        Ok(0)
    }
}

fn catalog_metadata_bytes(store: &CasStore) -> Result<u64, ClewError> {
    let mut bytes = optional_file_len(&store.catalog, "head.json")?;
    for directory in [&store.catalog_snapshots, &store.catalog_records] {
        for name in directory.entries()? {
            if directory.entry_kind(&name)? != ManagedEntryKind::File {
                return Err(corrupt("CAS catalog metadata contains a directory"));
            }
            bytes = bytes.saturating_add(directory.file_len(&name)?);
        }
    }
    Ok(bytes)
}

fn is_reclaimable_orphan_pack_file(name: &str) -> bool {
    if name.starts_with(".pack-") || name.starts_with(".tmp-") {
        return true;
    }
    ["pack", "json", "verified"].iter().any(|extension| {
        name.strip_suffix(&format!(".{extension}"))
            .is_some_and(|component| is_lower_hex(component, 64))
    })
}

fn is_lower_hex(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

fn acquire_world_lease(locks: &ManagedDirectory, mode: LockMode) -> Result<File, ClewError> {
    let file = locks.open_lock(OsStr::new("cas-world.lock"))?;
    #[cfg(unix)]
    {
        let operation = match mode {
            LockMode::Shared => libc::LOCK_SH,
            LockMode::Exclusive => libc::LOCK_EX,
        };
        if unsafe { libc::flock(file.as_raw_fd(), operation) } != 0 {
            return Err(io_error(std::io::Error::last_os_error()));
        }
    }
    Ok(file)
}

fn catalog_state_from_snapshot(
    snapshot: &CatalogSnapshot,
    snapshot_digest: &str,
) -> Result<CatalogState, ClewError> {
    let mut packs = BTreeMap::new();
    let mut previous = None;
    for pack in &snapshot.packs {
        if previous.is_some_and(|name: &str| name >= pack.data_name.as_str()) {
            return Err(corrupt(
                "CAS catalog snapshot packs are not strictly sorted",
            ));
        }
        validate_pack_manifest(&pack.data_name, &pack.manifest)?;
        if packs
            .insert(pack.data_name.clone(), pack.manifest.clone())
            .is_some()
        {
            return Err(corrupt("CAS catalog snapshot repeats a pack"));
        }
        previous = Some(pack.data_name.as_str());
    }
    let locations = locations_from_packs(&packs)?;
    Ok(CatalogState {
        initialized: true,
        sequence: snapshot.sequence,
        snapshot_sequence: snapshot.sequence,
        last_record_digest: snapshot.last_record_digest.clone(),
        snapshot_digest: Some(snapshot_digest.into()),
        tail_bytes: 0,
        packs,
        locations,
    })
}

fn catalog_snapshot_due(state: &CatalogState) -> bool {
    state.sequence.saturating_sub(state.snapshot_sequence) >= CATALOG_SNAPSHOT_INTERVAL
        || state.tail_bytes >= CATALOG_SNAPSHOT_TAIL_BYTES
}

fn snapshot_from_catalog_state(state: &CatalogState) -> CatalogSnapshot {
    CatalogSnapshot {
        schema: CATALOG_SNAPSHOT_SCHEMA.into(),
        sequence: state.sequence,
        last_record_digest: state.last_record_digest.clone(),
        packs: state
            .packs
            .iter()
            .map(|(data_name, manifest)| CatalogPack {
                data_name: data_name.clone(),
                manifest: manifest.clone(),
            })
            .collect(),
    }
}

fn apply_catalog_record(state: &mut CatalogState, record: &CatalogRecord) -> Result<(), ClewError> {
    validate_pack_manifest(&record.pack.data_name, &record.pack.manifest)?;
    match record.operation {
        CatalogOperation::Add => {
            if state
                .packs
                .insert(record.pack.data_name.clone(), record.pack.manifest.clone())
                .is_some()
            {
                return Err(corrupt("CAS catalog adds an existing pack"));
            }
        }
        CatalogOperation::Remove => {
            if state.packs.get(&record.pack.data_name) != Some(&record.pack.manifest) {
                return Err(corrupt("CAS catalog removes a different or missing pack"));
            }
            state.packs.remove(&record.pack.data_name);
        }
    }
    state.locations = locations_from_packs(&state.packs)?;
    Ok(())
}

fn locations_from_packs(
    packs: &BTreeMap<String, PackManifest>,
) -> Result<BTreeMap<String, PackLocation>, ClewError> {
    let mut locations = BTreeMap::new();
    for (data_name, manifest) in packs {
        add_pack_to_catalog(&mut locations, data_name, manifest)?;
    }
    Ok(locations)
}

fn validate_pack_manifest(data_name: &str, manifest: &PackManifest) -> Result<(), ClewError> {
    if manifest.schema != CAS_PACK_SCHEMA {
        return Err(corrupt("CAS pack schema is invalid"));
    }
    let manifest_digest = canonical::hash(manifest).map_err(internal)?;
    let component = digest_component(&manifest_digest)?;
    if data_name != format!("{component}.pack") {
        return Err(corrupt("CAS pack data name differs from its manifest"));
    }
    digest_component(&manifest.data_sha256)?;
    let mut end = 0u64;
    for entry in &manifest.objects {
        validate_reference(&entry.object)?;
        if entry.offset != end {
            return Err(corrupt("CAS pack object offsets are not contiguous"));
        }
        end = end
            .checked_add(entry.object.size)
            .ok_or_else(|| corrupt("CAS pack object range overflow"))?;
    }
    if end != manifest.data_size {
        return Err(corrupt("CAS pack object ranges do not cover pack data"));
    }
    Ok(())
}

fn catalog_record_name(sequence: u64, digest: &str) -> Result<String, ClewError> {
    Ok(format!(
        "{sequence:020}-{}.record",
        digest_component(digest)?
    ))
}

fn catalog_record_sequence(name: &OsStr) -> Option<u64> {
    let name = name.to_str()?;
    let (sequence, suffix) = name.split_once('-')?;
    if sequence.len() != 20 || !suffix.ends_with(".record") {
        return None;
    }
    sequence.parse().ok()
}

fn add_pack_to_catalog(
    catalog: &mut BTreeMap<String, PackLocation>,
    data_name: &str,
    manifest: &PackManifest,
) -> Result<(), ClewError> {
    for entry in &manifest.objects {
        let location = PackLocation {
            data_name: data_name.to_owned(),
            entry: entry.clone(),
        };
        match catalog.get(&entry.object.digest) {
            Some(existing) if existing.entry.object != entry.object => {
                return Err(corrupt("CAS pack catalog has a digest collision"));
            }
            Some(existing) if existing.data_name <= location.data_name => {}
            _ => {
                catalog.insert(entry.object.digest.clone(), location);
            }
        }
    }
    Ok(())
}

fn pack_verification_receipt(
    manifest_digest: &str,
    manifest: &PackManifest,
    metadata: &std::fs::Metadata,
) -> PackVerificationReceipt {
    PackVerificationReceipt {
        schema: PACK_VERIFICATION_SCHEMA.into(),
        manifest_digest: manifest_digest.into(),
        data_sha256: manifest.data_sha256.clone(),
        data_size: metadata.len(),
        device: metadata.dev(),
        inode: metadata.ino(),
        modified_seconds: metadata.mtime(),
        modified_nanoseconds: metadata.mtime_nsec(),
        changed_seconds: metadata.ctime(),
        changed_nanoseconds: metadata.ctime_nsec(),
    }
}

#[derive(Debug, Clone, Copy)]
enum LockMode {
    Shared,
    Exclusive,
}

fn object_digest(object_schema: &str, bytes: &[u8]) -> String {
    let mut digest = Sha256::new();
    digest.update(CAS_DOMAIN);
    digest.update(object_schema.as_bytes());
    digest.update([0]);
    digest.update(bytes);
    format!("sha256:{}", hex::encode(digest.finalize()))
}

fn validate_reference(object: &CasObject) -> Result<(), ClewError> {
    if object.schema != CAS_OBJECT_SCHEMA || object.size > usize::MAX as u64 {
        return Err(corrupt("CAS object reference is invalid"));
    }
    validate_object_schema(&object.object_schema)?;
    digest_component(&object.digest).map(|_| ())
}

fn validate_object_schema(value: &str) -> Result<(), ClewError> {
    if value.is_empty()
        || value.len() > 128
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'/'))
    {
        return Err(ClewError::new(
            ErrorCode::InvalidInput,
            "CAS object schema is not a bounded canonical identifier",
        ));
    }
    Ok(())
}

fn digest_component(value: &str) -> Result<&str, ClewError> {
    let component = value
        .strip_prefix("sha256:")
        .ok_or_else(|| corrupt("CAS digest prefix is invalid"))?;
    if component.len() != 64
        || !component
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Err(corrupt("CAS digest spelling is invalid"));
    }
    Ok(component)
}

fn corrupt(message: &str) -> ClewError {
    ClewError::new(ErrorCode::StateCorrupt, message)
}

fn io_error(error: std::io::Error) -> ClewError {
    ClewError::new(ErrorCode::Internal, error.to_string())
}

fn internal(error: impl std::fmt::Display) -> ClewError {
    ClewError::new(ErrorCode::Internal, error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::Arc;

    #[cfg(unix)]
    use std::os::unix::fs::symlink;

    fn store() -> (tempfile::TempDir, CasStore) {
        let root = tempfile::tempdir().unwrap();
        let authority = StateAuthority::open(root.path().join("v2")).unwrap();
        let store = CasStore::open(&authority).unwrap();
        (root, store)
    }

    #[test]
    fn identical_content_has_one_stable_object_identity() {
        let (_root, store) = store();
        let first = store.put("test/facts/1", b"same bytes").unwrap();
        let second = store.put("test/facts/1", b"same bytes").unwrap();
        assert_eq!(first, second);
        assert_eq!(store.read(&first, 1024).unwrap().bytes(), b"same bytes");
    }

    #[test]
    fn concurrent_batch_preserves_input_order_and_durability() {
        let (_root, store) = store();
        let objects = (0..128)
            .map(|index| {
                (
                    "test/batch/1".to_owned(),
                    format!("payload-{index}").into_bytes(),
                )
            })
            .collect::<Vec<_>>();
        let references = store.put_batch(objects.clone()).unwrap();
        assert_eq!(references.len(), objects.len());
        for ((schema, bytes), reference) in objects.iter().zip(&references) {
            assert_eq!(&reference.object_schema, schema);
            assert_eq!(store.read(reference, 1024).unwrap().bytes(), bytes);
        }
    }

    #[test]
    fn large_batch_stays_one_pack_without_loose_metadata_fanout() {
        let (_root, store) = store();
        let objects = (0..4096)
            .map(|index| {
                (
                    "test/large-batch/1".to_owned(),
                    format!("fact-{index:04}").into_bytes(),
                )
            })
            .collect::<Vec<_>>();
        let first = store.put_batch(objects.clone()).unwrap();
        assert!(store.objects.entries().unwrap().is_empty());
        assert_eq!(
            store
                .packs
                .entries()
                .unwrap()
                .into_iter()
                .filter(|name| Path::new(name).extension() == Some(OsStr::new("pack")))
                .count(),
            1
        );
        for index in [0, 1023, 2047, 4095] {
            assert_eq!(
                store.read(&first[index], 1024).unwrap().bytes(),
                objects[index].1
            );
        }
        let second = store.put_batch(objects).unwrap();
        assert_eq!(first, second);
        assert_eq!(
            store
                .packs
                .entries()
                .unwrap()
                .into_iter()
                .filter(|name| Path::new(name).extension() == Some(OsStr::new("pack")))
                .count(),
            1
        );
    }

    #[test]
    fn packed_batch_reads_without_creating_loose_objects() {
        let (root, store) = store();
        let object = store
            .put_batch(vec![("test/packed/1".into(), b"durable payload".to_vec())])
            .unwrap()
            .remove(0);
        let loose = store.object_path(&object.digest).unwrap();
        assert!(!loose.exists());
        assert_eq!(
            store.read(&object, 1024).unwrap().bytes(),
            b"durable payload"
        );
        assert!(!loose.exists());

        let reopened = CasStore::open_with_catalog(
            &StateAuthority::open(root.path().join("v2")).unwrap(),
            false,
        )
        .unwrap();
        assert_eq!(
            reopened.read(&object, 1024).unwrap().bytes(),
            b"durable payload"
        );
        assert!(!loose.exists());
    }

    #[test]
    fn pack_reopen_uses_snapshot_without_global_data_verification() {
        let (root, store) = store();
        let object = store
            .put_batch(vec![(
                "test/verified-pack/1".into(),
                vec![b'x'; 4 * 1024 * 1024],
            )])
            .unwrap()
            .remove(0);
        assert_eq!(
            store
                .full_pack_verifications
                .load(std::sync::atomic::Ordering::Relaxed),
            1
        );
        assert!(!store.packs.entries().unwrap().into_iter().any(|name| {
            matches!(
                Path::new(&name).extension().and_then(OsStr::to_str),
                Some("json" | "verified")
            )
        }));
        let reopened = CasStore::open_with_catalog(
            &StateAuthority::open(root.path().join("v2")).unwrap(),
            false,
        )
        .unwrap();
        assert_eq!(
            reopened
                .full_pack_verifications
                .load(std::sync::atomic::Ordering::Relaxed),
            0
        );
        assert_eq!(
            reopened.read(&object, 4 * 1024 * 1024).unwrap().bytes(),
            vec![b'x'; 4 * 1024 * 1024]
        );
    }

    #[test]
    fn packed_batch_never_returns_tampered_data() {
        let (_root, store) = store();
        let object = store
            .put_batch(vec![("test/packed/1".into(), b"trusted payload".to_vec())])
            .unwrap()
            .remove(0);
        let pack = fs::read_dir(store.packs.path())
            .unwrap()
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .find(|path| path.extension() == Some(OsStr::new("pack")))
            .unwrap();
        fs::write(pack, b"forged payload").unwrap();
        assert_eq!(
            store.read(&object, 1024).unwrap_err().code,
            ErrorCode::StateCorrupt
        );
    }

    #[test]
    fn reopening_never_reads_same_size_tampered_pack() {
        let (root, store) = store();
        store
            .put_batch(vec![("test/packed/1".into(), b"trusted payload".to_vec())])
            .unwrap();
        let pack = fs::read_dir(store.packs.path())
            .unwrap()
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .find(|path| path.extension() == Some(OsStr::new("pack")))
            .unwrap();
        fs::write(pack, b"forged! payload").unwrap();

        let reopened = CasStore::open_with_catalog(
            &StateAuthority::open(root.path().join("v2")).unwrap(),
            false,
        )
        .unwrap();
        let object = store
            .pack_catalog
            .read()
            .unwrap()
            .locations
            .values()
            .next()
            .unwrap()
            .entry
            .object
            .clone();
        assert_eq!(
            reopened.read(&object, 1024).unwrap_err().code,
            ErrorCode::StateCorrupt
        );
    }

    #[test]
    fn durable_catalog_retires_redundant_pack_index() {
        let (root, store) = store();
        let input = vec![("test/recovery/1".into(), b"durable payload".to_vec())];
        let object = store.put_batch(input.clone()).unwrap().remove(0);
        assert!(
            !fs::read_dir(store.packs.path())
                .unwrap()
                .filter_map(Result::ok)
                .any(|entry| entry.path().extension() == Some(OsStr::new("json")))
        );

        let reopened = CasStore::open_with_catalog(
            &StateAuthority::open(root.path().join("v2")).unwrap(),
            false,
        )
        .unwrap();
        assert_eq!(reopened.put_batch(input).unwrap(), vec![object.clone()]);
        assert_eq!(
            reopened.read(&object, 1024).unwrap().bytes(),
            b"durable payload"
        );
        assert_eq!(
            reopened.read(&object, 1024).unwrap().bytes(),
            b"durable payload"
        );
    }

    #[test]
    fn first_catalog_open_migrates_existing_pack_indexes_once() {
        let (root, store) = store();
        let object = store
            .put_batch(vec![(
                "test/catalog-migration/1".into(),
                b"existing".to_vec(),
            )])
            .unwrap()
            .remove(0);
        let (data_name, manifest) = {
            let state = store.pack_catalog.read().unwrap();
            let (name, manifest) = state.packs.iter().next().unwrap();
            (name.clone(), manifest.clone())
        };
        let component = Path::new(&data_name)
            .file_stem()
            .and_then(OsStr::to_str)
            .unwrap();
        store
            .packs
            .atomic_write(
                OsStr::new(&format!("{component}.json")),
                &canonical::bytes(&manifest).unwrap(),
            )
            .unwrap();
        store.catalog.remove_file(OsStr::new("head.json")).unwrap();
        for name in store.catalog_snapshots.entries().unwrap() {
            store.catalog_snapshots.remove_file(&name).unwrap();
        }
        for name in store.catalog_records.entries().unwrap() {
            store.catalog_records.remove_file(&name).unwrap();
        }
        drop(store);

        let migrated = CasStore::open_with_catalog(
            &StateAuthority::open(root.path().join("v2")).unwrap(),
            false,
        )
        .unwrap();
        assert_eq!(
            migrated
                .catalog_bootstrap_scans
                .load(std::sync::atomic::Ordering::Relaxed),
            1
        );
        assert_eq!(migrated.read(&object, 1024).unwrap().bytes(), b"existing");
        assert!(
            !migrated
                .packs
                .file_exists(OsStr::new(&format!("{component}.json")))
                .unwrap()
        );
    }

    #[test]
    fn existing_reader_refreshes_catalog_after_another_store_publishes() {
        let (root, first) = store();
        let second =
            CasStore::open(&StateAuthority::open(root.path().join("v2")).unwrap()).unwrap();
        let object = second
            .put_batch(vec![(
                "test/visibility/1".into(),
                b"published later".to_vec(),
            )])
            .unwrap()
            .remove(0);

        assert_eq!(
            first.read(&object, 1024).unwrap().bytes(),
            b"published later"
        );
    }

    #[test]
    fn independent_reader_applies_only_new_catalog_tail_on_miss() {
        let (root, first) = store();
        assert_eq!(
            first
                .catalog_bootstrap_scans
                .load(std::sync::atomic::Ordering::Relaxed),
            1
        );
        let authority = StateAuthority::open(root.path().join("v2")).unwrap();
        let independent = CasStore::open_with_catalog(&authority, false).unwrap();
        assert_eq!(
            independent
                .catalog_bootstrap_scans
                .load(std::sync::atomic::Ordering::Relaxed),
            0
        );
        let object = first
            .put_batch(vec![("test/tail/1".into(), b"new tail".to_vec())])
            .unwrap()
            .remove(0);

        assert_eq!(
            independent.read(&object, 1024).unwrap().bytes(),
            b"new tail"
        );
        assert_eq!(
            independent
                .catalog_bootstrap_scans
                .load(std::sync::atomic::Ordering::Relaxed),
            0
        );
    }

    #[test]
    fn periodic_snapshot_bounds_catalog_metadata() {
        let (_root, store) = store();
        for index in 0..CATALOG_SNAPSHOT_INTERVAL {
            store
                .put_batch(vec![(
                    "test/snapshot/1".into(),
                    format!("payload-{index}").into_bytes(),
                )])
                .unwrap();
        }

        assert!(store.catalog_records.entries().unwrap().is_empty());
        assert_eq!(
            store
                .catalog_snapshots
                .entries()
                .unwrap()
                .into_iter()
                .filter(|name| Path::new(name).extension() == Some(OsStr::new("snapshot")))
                .count(),
            1
        );
    }

    #[test]
    fn catalog_snapshot_is_due_by_count_or_tail_bytes() {
        let mut state = CatalogState {
            sequence: CATALOG_SNAPSHOT_INTERVAL,
            ..CatalogState::default()
        };
        assert!(catalog_snapshot_due(&state));

        state.sequence = 1;
        state.tail_bytes = CATALOG_SNAPSHOT_TAIL_BYTES;
        assert!(catalog_snapshot_due(&state));

        state.tail_bytes = CATALOG_SNAPSHOT_TAIL_BYTES - 1;
        assert!(!catalog_snapshot_due(&state));
    }

    #[test]
    fn corrupt_catalog_record_fails_closed() {
        let (root, store) = store();
        store
            .put_batch(vec![("test/catalog-corrupt/1".into(), b"trusted".to_vec())])
            .unwrap();
        let record = fs::read_dir(store.catalog_records.path())
            .unwrap()
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .find(|path| path.extension() == Some(OsStr::new("record")))
            .unwrap();
        fs::write(record, b"{}\n").unwrap();
        drop(store);

        assert_eq!(
            CasStore::open_with_catalog(
                &StateAuthority::open(root.path().join("v2")).unwrap(),
                false,
            )
            .unwrap_err()
            .code,
            ErrorCode::StateCorrupt
        );
    }

    #[test]
    fn next_catalog_sync_prunes_crash_orphan_metadata() {
        let (root, store) = store();
        store
            .catalog_snapshots
            .atomic_write(OsStr::new("orphan.snapshot"), b"orphan")
            .unwrap();
        store
            .catalog_records
            .atomic_write(OsStr::new(".tmp-crash"), b"partial")
            .unwrap();

        CasStore::open_with_catalog(
            &StateAuthority::open(root.path().join("v2")).unwrap(),
            false,
        )
        .unwrap();
        assert!(
            !store
                .catalog_snapshots
                .file_exists(OsStr::new("orphan.snapshot"))
                .unwrap()
        );
        assert!(
            !store
                .catalog_records
                .file_exists(OsStr::new(".tmp-crash"))
                .unwrap()
        );
    }

    #[test]
    fn storage_gc_reclaims_only_transitively_unreachable_objects() {
        let root = tempfile::tempdir().unwrap();
        let authority = StateAuthority::open(root.path().join("v2")).unwrap();
        let store = CasStore::open(&authority).unwrap();
        let live_pack = store
            .put_batch(vec![("test/storage-pack/1".into(), b"live pack".to_vec())])
            .unwrap()
            .remove(0);
        let dead_pack = store
            .put_batch(vec![("test/storage-pack/1".into(), b"dead pack".to_vec())])
            .unwrap()
            .remove(0);
        let live_loose = store.put("test/storage-loose/1", b"live loose").unwrap();
        let dead_loose = store.put("test/storage-loose/1", b"dead loose").unwrap();
        authority
            .write_private_atomic(
                &authority.root().join("sessions/storage-root.json"),
                &canonical::bytes(&serde_json::json!({
                    "schema":"test-storage-root/1.0",
                    "pack":live_pack,
                    "loose":live_loose,
                }))
                .unwrap(),
            )
            .unwrap();

        let report = storage_status(&authority).unwrap();
        assert_eq!(report.action, StorageAction::DryRun);
        assert_eq!(report.reclaimable_packs, 1);
        assert_eq!(report.reclaimable_loose_objects, 1);
        assert!(report.reclaimable_bytes >= dead_pack.size + dead_loose.size);
        drop(store);

        let applied = garbage_collect_storage(&authority).unwrap();
        assert_eq!(applied.action, StorageAction::Applied);
        assert_eq!(applied.reclaimed_bytes, applied.reclaimable_bytes);
        let reopened = CasStore::open(&authority).unwrap();
        assert_eq!(
            reopened.read(&live_pack, 1024).unwrap().bytes(),
            b"live pack"
        );
        assert_eq!(
            reopened.read(&live_loose, 1024).unwrap().bytes(),
            b"live loose"
        );
        assert_eq!(
            reopened.read(&dead_pack, 1024).unwrap_err().code,
            ErrorCode::StateCorrupt
        );
        assert_eq!(
            reopened.read(&dead_loose, 1024).unwrap_err().code,
            ErrorCode::StateCorrupt
        );
        assert_eq!(storage_status(&authority).unwrap().reclaimable_bytes, 0);
    }

    #[test]
    fn storage_gc_retains_a_pack_when_any_member_is_reachable() {
        let root = tempfile::tempdir().unwrap();
        let authority = StateAuthority::open(root.path().join("v2")).unwrap();
        let store = CasStore::open(&authority).unwrap();
        let objects = store
            .put_batch(vec![
                ("test/mixed-pack/1".into(), b"live".to_vec()),
                ("test/mixed-pack/1".into(), b"dead".to_vec()),
            ])
            .unwrap();
        authority
            .write_private_atomic(
                &authority.root().join("sessions/mixed-root.json"),
                &canonical::bytes(&serde_json::json!({"live":objects[0]})).unwrap(),
            )
            .unwrap();

        let report = storage_status(&authority).unwrap();
        assert_eq!(report.reclaimable_packs, 0);
        assert_eq!(report.retained_mixed_packs, 1);
    }

    #[test]
    fn storage_gc_refuses_to_guess_through_a_corrupt_root() {
        let root = tempfile::tempdir().unwrap();
        let authority = StateAuthority::open(root.path().join("v2")).unwrap();
        let _store = CasStore::open(&authority).unwrap();
        authority
            .write_private_atomic(
                &authority.root().join("sessions/corrupt.json"),
                b"{not-json",
            )
            .unwrap();

        assert_eq!(
            storage_status(&authority).unwrap_err().code,
            ErrorCode::StateCorrupt
        );
    }

    #[test]
    fn storage_reachability_follows_nested_cas_references() {
        let root = tempfile::tempdir().unwrap();
        let authority = StateAuthority::open(root.path().join("v2")).unwrap();
        let store = CasStore::open(&authority).unwrap();
        let leaf = store.put("test/reachable-leaf/1", b"leaf").unwrap();
        let parent_bytes = canonical::bytes(&serde_json::json!({
            "schema":"test-parent/1.0",
            "leaf":leaf,
        }))
        .unwrap();
        let parent = store.put("test/reachable-parent/1", &parent_bytes).unwrap();
        authority
            .write_private_atomic(
                &authority.root().join("sessions/transitive-root.json"),
                &canonical::bytes(&serde_json::json!({"parent":parent})).unwrap(),
            )
            .unwrap();

        let report = storage_status(&authority).unwrap();
        assert_eq!(report.reachable_objects, 2);
        assert_eq!(report.reclaimable_loose_objects, 0);
    }

    #[test]
    fn active_store_lease_delays_physical_gc() {
        let root = tempfile::tempdir().unwrap();
        let authority = StateAuthority::open(root.path().join("v2")).unwrap();
        let store = CasStore::open(&authority).unwrap();
        store.put("test/lease/1", b"unrooted").unwrap();
        let (sent, received) = std::sync::mpsc::channel();
        let worker_authority = authority.clone();
        let worker = std::thread::spawn(move || {
            let result = garbage_collect_storage(&worker_authority);
            sent.send(result).unwrap();
        });

        assert!(
            received
                .recv_timeout(std::time::Duration::from_millis(100))
                .is_err()
        );
        drop(store);
        assert!(
            received
                .recv_timeout(std::time::Duration::from_secs(5))
                .unwrap()
                .is_ok()
        );
        worker.join().unwrap();
    }

    #[test]
    fn equal_pack_bytes_with_different_object_boundaries_do_not_collide() {
        let (_root, store) = store();
        let first = store
            .put_batch(vec![
                ("test/boundary/1".into(), b"ab".to_vec()),
                ("test/boundary/1".into(), b"c".to_vec()),
            ])
            .unwrap();
        let second = store
            .put_batch(vec![
                ("test/boundary/1".into(), b"a".to_vec()),
                ("test/boundary/1".into(), b"bc".to_vec()),
            ])
            .unwrap();

        assert_eq!(
            store
                .packs
                .entries()
                .unwrap()
                .into_iter()
                .filter(|name| Path::new(name).extension() == Some(OsStr::new("pack")))
                .count(),
            2
        );
        for (object, expected) in first.iter().zip([b"ab".as_slice(), b"c".as_slice()]) {
            assert_eq!(store.read(object, 1024).unwrap().bytes(), expected);
        }
        for (object, expected) in second.iter().zip([b"a".as_slice(), b"bc".as_slice()]) {
            assert_eq!(store.read(object, 1024).unwrap().bytes(), expected);
        }
    }

    #[test]
    fn schema_is_part_of_the_digest_domain() {
        let (_root, store) = store();
        let left = store.put("test/left/1", b"same bytes").unwrap();
        let right = store.put("test/right/1", b"same bytes").unwrap();
        assert_ne!(left.digest, right.digest);
    }

    #[test]
    fn corrupt_object_is_quarantined_and_never_returned() {
        let (root, store) = store();
        let object = store.put("test/facts/1", b"trusted").unwrap();
        let path = store.object_path(&object.digest).unwrap();
        fs::write(&path, b"forged!").unwrap();
        let error = store.read(&object, 1024).unwrap_err();
        assert_eq!(error.code, ErrorCode::StateCorrupt);
        assert!(!path.exists());
        assert!(
            fs::read_dir(root.path().join("v2/quarantine"))
                .unwrap()
                .next()
                .is_some()
        );
    }

    #[test]
    fn concurrent_publishers_converge_on_one_object() {
        let (root, store) = store();
        let store = Arc::new(store);
        let threads = (0..8)
            .map(|_| {
                let store = Arc::clone(&store);
                std::thread::spawn(move || store.put("test/facts/1", b"parallel").unwrap())
            })
            .collect::<Vec<_>>();
        let objects = threads
            .into_iter()
            .map(|thread| thread.join().unwrap())
            .collect::<Vec<_>>();
        assert!(root.path().exists());
        assert!(objects.windows(2).all(|pair| pair[0] == pair[1]));
    }

    #[test]
    fn read_budget_is_fail_closed() {
        let (_root, store) = store();
        let object = store.put("test/facts/1", b"0123456789").unwrap();
        assert_eq!(
            store.read(&object, 4).unwrap_err().code,
            ErrorCode::ResourceLimit
        );
    }

    #[cfg(unix)]
    #[test]
    fn replaced_state_root_cannot_redirect_cas_publication() {
        let (root, store) = store();
        let state_root = root.path().join("v2");
        let pinned_root = root.path().join("pinned-v2");
        fs::rename(&state_root, &pinned_root).unwrap();
        fs::create_dir(&state_root).unwrap();

        let object = store
            .put_batch(vec![("test/root-swap/1".into(), b"trusted".to_vec())])
            .unwrap()
            .remove(0);
        assert_eq!(store.read(&object, 1024).unwrap().bytes(), b"trusted");
        assert!(
            fs::read_dir(pinned_root.join("objects/packs-v3"))
                .unwrap()
                .next()
                .is_some()
        );
        assert!(fs::read_dir(&state_root).unwrap().next().is_none());
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_object_prefix_cannot_escape_cas_authority() {
        let (root, store) = store();
        let bytes = b"trusted";
        let digest = object_digest("test/symlink/1", bytes);
        let hex = digest_component(&digest).unwrap();
        let prefix = root.path().join("v2/objects/sha256").join(&hex[..2]);
        let victim = root.path().join("victim");
        fs::create_dir(&victim).unwrap();
        symlink(&victim, prefix).unwrap();

        assert!(store.put("test/symlink/1", bytes).is_err());
        assert!(fs::read_dir(victim).unwrap().next().is_none());
    }
}
