use crate::canonical;
use crate::error::{ClewError, ErrorCode};
use crate::state::{ManagedDirectory, StateAuthority};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsStr;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::os::unix::fs::MetadataExt;
use std::path::Path;
use std::sync::{Arc, RwLock};

#[cfg(unix)]
use std::os::fd::AsRawFd;

const CAS_DOMAIN: &[u8] = b"codeclew-cas/v2\0";
pub const CAS_OBJECT_SCHEMA: &str = "codeclew-cas-object/2.0";
const CAS_PACK_SCHEMA: &str = "codeclew-cas-pack/3.0";
const PACK_VERIFICATION_SCHEMA: &str = "codeclew-cas-pack-verification/1.0";
const MAX_PACK_INDEX_BYTES: usize = 64 * 1024 * 1024;
const MAX_PACK_VERIFICATION_BYTES: usize = 4096;

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
    locks: ManagedDirectory,
    quarantine: ManagedDirectory,
    pack_catalog: Arc<RwLock<BTreeMap<String, PackLocation>>>,
    #[cfg(test)]
    full_pack_verifications: Arc<std::sync::atomic::AtomicUsize>,
}

impl CasStore {
    pub fn open(authority: &StateAuthority) -> Result<Self, ClewError> {
        let store = Self {
            objects: authority.directory(Path::new("objects/sha256"))?,
            packs: authority.directory(Path::new("objects/packs-v3"))?,
            locks: authority.directory(Path::new("locks"))?,
            quarantine: authority.directory(Path::new("quarantine"))?,
            pack_catalog: Arc::new(RwLock::new(BTreeMap::new())),
            #[cfg(test)]
            full_pack_verifications: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
        };
        store.refresh_pack_catalog()?;
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
        self.refresh_pack_catalog()?;
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
                self.refresh_pack_catalog()?;
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
        self.install_pack_manifest(&data_name, &manifest)?;
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

    fn refresh_pack_catalog(&self) -> Result<(), ClewError> {
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
        let mut catalog = BTreeMap::new();
        for (data_name, manifest) in verified {
            add_pack_to_catalog(&mut catalog, &data_name, &manifest)?;
        }
        *self
            .pack_catalog
            .write()
            .map_err(|_| internal("CAS pack catalog lock is poisoned"))? = catalog;
        Ok(())
    }

    fn install_pack_manifest(
        &self,
        data_name: &str,
        manifest: &PackManifest,
    ) -> Result<(), ClewError> {
        let mut catalog = self
            .pack_catalog
            .write()
            .map_err(|_| internal("CAS pack catalog lock is poisoned"))?;
        add_pack_to_catalog(&mut catalog, data_name, manifest)
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
        if manifest.schema != CAS_PACK_SCHEMA
            || canonical::bytes(&manifest).map_err(internal)? != bytes
            || expected.is_some_and(|expected| expected != &manifest)
        {
            return Err(corrupt("CAS pack authority mismatch"));
        }
        let manifest_digest = canonical::hash(&manifest).map_err(internal)?;
        let component = digest_component(&manifest_digest)?;
        if Path::new(data_name).file_stem().and_then(OsStr::to_str) != Some(component) {
            return Err(corrupt("CAS pack data name differs from its manifest"));
        }
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

        let reopened =
            CasStore::open(&StateAuthority::open(root.path().join("v2")).unwrap()).unwrap();
        assert_eq!(
            reopened.read(&object, 1024).unwrap().bytes(),
            b"durable payload"
        );
        assert!(!loose.exists());
    }

    #[test]
    fn unchanged_pack_reopen_uses_metadata_bound_verification_receipt() {
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
        let reopened =
            CasStore::open(&StateAuthority::open(root.path().join("v2")).unwrap()).unwrap();
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
    fn reopening_rejects_same_size_tampered_pack() {
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

        assert_eq!(
            CasStore::open(&StateAuthority::open(root.path().join("v2")).unwrap())
                .unwrap_err()
                .code,
            ErrorCode::StateCorrupt
        );
    }

    #[test]
    fn identical_retry_recovers_pack_with_missing_index() {
        let (root, store) = store();
        let input = vec![("test/recovery/1".into(), b"durable payload".to_vec())];
        let object = store.put_batch(input.clone()).unwrap().remove(0);
        let index = fs::read_dir(store.packs.path())
            .unwrap()
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .find(|path| path.extension() == Some(OsStr::new("json")))
            .unwrap();
        fs::remove_file(index).unwrap();

        let reopened =
            CasStore::open(&StateAuthority::open(root.path().join("v2")).unwrap()).unwrap();
        assert_eq!(reopened.put_batch(input).unwrap(), vec![object.clone()]);
        assert_eq!(
            reopened.read(&object, 1024).unwrap().bytes(),
            b"durable payload"
        );
        assert_eq!(
            reopened
                .packs
                .entries()
                .unwrap()
                .into_iter()
                .filter(|name| Path::new(name).extension() == Some(OsStr::new("json")))
                .count(),
            1
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
