use crate::canonical;
use crate::error::{ClewError, ErrorCode};
use crate::state::{StateAuthority, create_private_directory};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::ffi::OsStr;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

#[cfg(unix)]
use std::os::fd::AsRawFd;
#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

const CAS_DOMAIN: &[u8] = b"codeclew-cas/v2\0";
pub const CAS_OBJECT_SCHEMA: &str = "codeclew-cas-object/2.0";
const CAS_PACK_SCHEMA: &str = "codeclew-cas-pack/2.0";
const MAX_PACK_INDEX_BYTES: usize = 64 * 1024 * 1024;

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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CasObject {
    pub schema: String,
    pub object_schema: String,
    pub digest: String,
    pub size: u64,
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
    objects: PathBuf,
    packs: PathBuf,
    locks: PathBuf,
    quarantine: PathBuf,
}

impl CasStore {
    pub fn open(authority: &StateAuthority) -> Result<Self, ClewError> {
        let store = Self {
            objects: authority.objects_root(),
            packs: authority.packs_root(),
            locks: authority.locks_root(),
            quarantine: authority.quarantine_root(),
        };
        for path in [
            &store.objects,
            &store.packs,
            &store.locks,
            &store.quarantine,
        ] {
            create_private_directory(path)?;
        }
        Ok(store)
    }

    pub fn put(&self, object_schema: &str, bytes: &[u8]) -> Result<CasObject, ClewError> {
        validate_object_schema(object_schema)?;
        let digest = object_digest(object_schema, bytes);
        let object = CasObject {
            schema: CAS_OBJECT_SCHEMA.into(),
            object_schema: object_schema.into(),
            digest: digest.clone(),
            size: bytes.len() as u64,
        };
        let lock = self.lock(&digest, LockMode::Exclusive)?;
        let path = self.object_path(&digest)?;
        if path.exists() {
            match self.read_path(&object, &path, bytes.len()) {
                Ok(existing) if existing == bytes => return Ok(object),
                Ok(_) | Err(_) => self.quarantine_locked(&path, &digest)?,
            }
        }
        self.write_atomic(&path, bytes)?;
        let persisted = self.read_path(&object, &path, bytes.len())?;
        if persisted != bytes {
            return Err(corrupt("CAS object changed during atomic publication"));
        }
        drop(lock);
        Ok(object)
    }

    /// Publish an indexed batch as one durable pack while preserving the
    /// caller's deterministic input order. Loose files are a derived read
    /// cache and can be rebuilt from the sealed pack after a crash.
    pub fn put_batch(&self, objects: Vec<(String, Vec<u8>)>) -> Result<Vec<CasObject>, ClewError> {
        let mut prepared = Vec::with_capacity(objects.len());
        for (schema, bytes) in objects {
            validate_object_schema(&schema)?;
            let object = CasObject {
                schema: CAS_OBJECT_SCHEMA.into(),
                object_schema: schema,
                digest: String::new(),
                size: bytes.len() as u64,
            };
            let mut object = object;
            object.digest = object_digest(&object.object_schema, &bytes);
            prepared.push((object, bytes));
        }
        let references = prepared
            .iter()
            .map(|(object, _)| object.clone())
            .collect::<Vec<_>>();
        if prepared.is_empty() {
            return Ok(references);
        }
        let batch_lock = self.batch_lock()?;
        let mut missing = Vec::new();
        for (object, bytes) in &prepared {
            let path = self.object_path(&object.digest)?;
            if path.exists() {
                match self.read_path(object, &path, bytes.len()) {
                    Ok(existing) if existing == *bytes => continue,
                    Ok(_) | Err(_) => {
                        let lock = self.lock(&object.digest, LockMode::Exclusive)?;
                        if path.exists() {
                            self.quarantine_locked(&path, &object.digest)?;
                        }
                        drop(lock);
                    }
                }
            }
            missing.push((object.clone(), bytes.clone()));
        }
        if !missing.is_empty() {
            self.write_pack(&missing)?;
            missing
                .par_iter()
                .try_for_each(|(object, bytes)| self.materialize_loose(object, bytes))?;
            for (object, bytes) in &missing {
                let path = self.object_path(&object.digest)?;
                if self.read_path(object, &path, bytes.len())? != *bytes {
                    return Err(corrupt("CAS pack materialization changed object bytes"));
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
        let path = self.object_path(&object.digest)?;
        match self.read_path(object, &path, max_bytes) {
            Ok(bytes) => Ok(CasLease {
                object: object.clone(),
                bytes,
                lock,
            }),
            Err(error) => {
                drop(lock);
                let exclusive = self.lock(&object.digest, LockMode::Exclusive)?;
                if path.exists() {
                    self.quarantine_locked(&path, &object.digest)?;
                }
                if let Some(bytes) = self.read_from_packs(object, max_bytes)? {
                    self.materialize_loose(object, &bytes)?;
                    drop(exclusive);
                    let lock = self.lock(&object.digest, LockMode::Shared)?;
                    let bytes = self.read_path(object, &path, max_bytes)?;
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
        let temporary = self.packs.join(format!(".pack-{}", uuid::Uuid::new_v4()));
        let mut options = OpenOptions::new();
        options.create_new(true).write(true);
        #[cfg(unix)]
        options.mode(0o600);
        let mut file = options.open(&temporary).map_err(io_error)?;
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
        let component = digest_component(&data_sha256)?;
        let data_path = self.packs.join(format!("{component}.pack"));
        let index_path = self.packs.join(format!("{component}.json"));
        let manifest = PackManifest {
            schema: CAS_PACK_SCHEMA.into(),
            data_sha256,
            data_size: offset,
            objects: entries,
        };
        if data_path.exists() || index_path.exists() {
            let _ = fs::remove_file(&temporary);
            self.verify_pack_pair(&data_path, &index_path, Some(&manifest))?;
            return Ok(());
        }
        fs::rename(&temporary, &data_path).map_err(io_error)?;
        sync_directory(&self.packs)?;
        let bytes = canonical::bytes(&manifest).map_err(internal)?;
        if bytes.len() > MAX_PACK_INDEX_BYTES {
            return Err(ClewError::new(
                ErrorCode::ResourceLimit,
                "CAS pack index exceeds its bounded size",
            ));
        }
        self.write_atomic(&index_path, &bytes)?;
        self.verify_pack_pair(&data_path, &index_path, Some(&manifest))?;
        Ok(())
    }

    fn materialize_loose(&self, object: &CasObject, bytes: &[u8]) -> Result<(), ClewError> {
        if object_digest(&object.object_schema, bytes) != object.digest
            || bytes.len() as u64 != object.size
        {
            return Err(corrupt("CAS pack object does not match its reference"));
        }
        let path = self.object_path(&object.digest)?;
        if path.exists() && self.read_path(object, &path, bytes.len()).is_ok() {
            return Ok(());
        }
        let parent = path
            .parent()
            .ok_or_else(|| corrupt("CAS object has no parent"))?;
        create_private_directory(parent)?;
        let temporary = parent.join(format!(".derived-{}", uuid::Uuid::new_v4()));
        let mut options = OpenOptions::new();
        options.create_new(true).write(true);
        #[cfg(unix)]
        options.mode(0o600);
        let mut file = options.open(&temporary).map_err(io_error)?;
        file.write_all(bytes).map_err(io_error)?;
        file.flush().map_err(io_error)?;
        drop(file);
        fs::rename(&temporary, path).map_err(io_error)
    }

    fn read_from_packs(
        &self,
        object: &CasObject,
        max_bytes: usize,
    ) -> Result<Option<Vec<u8>>, ClewError> {
        let mut indexes = fs::read_dir(&self.packs)
            .map_err(io_error)?
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| path.extension() == Some(OsStr::new("json")))
            .collect::<Vec<_>>();
        indexes.sort();
        for index_path in indexes {
            let component = index_path
                .file_stem()
                .and_then(OsStr::to_str)
                .ok_or_else(|| corrupt("CAS pack index name is invalid"))?;
            let data_path = self.packs.join(format!("{component}.pack"));
            let manifest = self.verify_pack_pair(&data_path, &index_path, None)?;
            let Some(entry) = manifest
                .objects
                .iter()
                .find(|entry| entry.object.digest == object.digest)
            else {
                continue;
            };
            if &entry.object != object || object.size > max_bytes as u64 {
                return Err(corrupt("CAS pack reference differs from requested object"));
            }
            let mut file = OpenOptions::new()
                .read(true)
                .open(&data_path)
                .map_err(io_error)?;
            file.seek(SeekFrom::Start(entry.offset)).map_err(io_error)?;
            let mut bytes = vec![0; object.size as usize];
            file.read_exact(&mut bytes).map_err(io_error)?;
            if object_digest(&object.object_schema, &bytes) != object.digest {
                return Err(corrupt("CAS packed object digest mismatch"));
            }
            return Ok(Some(bytes));
        }
        Ok(None)
    }

    fn verify_pack_pair(
        &self,
        data_path: &Path,
        index_path: &Path,
        expected: Option<&PackManifest>,
    ) -> Result<PackManifest, ClewError> {
        let index_metadata =
            fs::symlink_metadata(index_path).map_err(|_| corrupt("CAS pack index is missing"))?;
        let data_metadata =
            fs::symlink_metadata(data_path).map_err(|_| corrupt("CAS pack data is missing"))?;
        if index_metadata.file_type().is_symlink()
            || data_metadata.file_type().is_symlink()
            || !index_metadata.is_file()
            || !data_metadata.is_file()
            || index_metadata.len() > MAX_PACK_INDEX_BYTES as u64
        {
            return Err(corrupt("CAS pack metadata is invalid"));
        }
        #[cfg(unix)]
        if index_metadata.permissions().mode() & 0o077 != 0
            || data_metadata.permissions().mode() & 0o077 != 0
        {
            return Err(corrupt("CAS pack permissions are not private"));
        }
        let bytes = fs::read(index_path).map_err(io_error)?;
        let manifest: PackManifest =
            serde_json::from_slice(&bytes).map_err(|_| corrupt("CAS pack index is invalid"))?;
        if manifest.schema != CAS_PACK_SCHEMA
            || canonical::bytes(&manifest).map_err(internal)? != bytes
            || manifest.data_size != data_metadata.len()
            || expected.is_some_and(|expected| expected != &manifest)
        {
            return Err(corrupt("CAS pack authority mismatch"));
        }
        let component = digest_component(&manifest.data_sha256)?;
        if data_path.file_stem().and_then(OsStr::to_str) != Some(component) {
            return Err(corrupt("CAS pack data name differs from its digest"));
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
        Ok(manifest)
    }

    fn read_path(
        &self,
        object: &CasObject,
        path: &Path,
        max_bytes: usize,
    ) -> Result<Vec<u8>, ClewError> {
        let metadata = fs::symlink_metadata(path).map_err(|_| corrupt("CAS object is missing"))?;
        if metadata.file_type().is_symlink()
            || !metadata.is_file()
            || metadata.len() != object.size
            || metadata.len() > max_bytes as u64
        {
            return Err(corrupt("CAS object metadata is invalid"));
        }
        #[cfg(unix)]
        if metadata.permissions().mode() & 0o077 != 0 {
            return Err(corrupt("CAS object permissions are not private"));
        }
        let file = OpenOptions::new().read(true).open(path).map_err(io_error)?;
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

    fn write_atomic(&self, path: &Path, bytes: &[u8]) -> Result<(), ClewError> {
        let parent = path
            .parent()
            .ok_or_else(|| corrupt("CAS object has no parent"))?;
        create_private_directory(parent)?;
        let temporary = parent.join(format!(".tmp-{}", uuid::Uuid::new_v4()));
        let mut options = OpenOptions::new();
        options.create_new(true).write(true);
        #[cfg(unix)]
        options.mode(0o600);
        let mut file = options.open(&temporary).map_err(io_error)?;
        file.write_all(bytes).map_err(io_error)?;
        file.sync_all().map_err(io_error)?;
        fs::rename(&temporary, path).map_err(io_error)?;
        sync_directory(parent)?;
        Ok(())
    }

    fn quarantine_locked(&self, path: &Path, digest: &str) -> Result<(), ClewError> {
        create_private_directory(&self.quarantine)?;
        let destination = self.quarantine.join(format!(
            "cas-{}-{}",
            digest
                .strip_prefix("sha256:")
                .ok_or_else(|| corrupt("CAS digest prefix is invalid"))?,
            uuid::Uuid::new_v4()
        ));
        fs::rename(path, &destination).map_err(io_error)?;
        sync_directory(&self.quarantine)?;
        if let Some(parent) = path.parent() {
            sync_directory(parent)?;
        }
        Ok(())
    }

    fn object_path(&self, digest: &str) -> Result<PathBuf, ClewError> {
        let hex = digest_component(digest)?;
        Ok(self.objects.join(&hex[..2]).join(&hex[2..]))
    }

    fn lock(&self, digest: &str, mode: LockMode) -> Result<File, ClewError> {
        let component = digest_component(digest)?;
        let path = self.locks.join(format!("cas-{component}.lock"));
        let mut options = OpenOptions::new();
        options.create(true).read(true).write(true);
        #[cfg(unix)]
        options.mode(0o600);
        let file = options.open(path).map_err(io_error)?;
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
        let path = self.locks.join("cas-pack-batch.lock");
        let mut options = OpenOptions::new();
        options.create(true).read(true).write(true);
        #[cfg(unix)]
        options.mode(0o600);
        let file = options.open(path).map_err(io_error)?;
        #[cfg(unix)]
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) } != 0 {
            return Err(io_error(std::io::Error::last_os_error()));
        }
        Ok(file)
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

fn sync_directory(path: &Path) -> Result<(), ClewError> {
    File::open(path)
        .map_err(io_error)?
        .sync_all()
        .map_err(io_error)
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
    use std::sync::Arc;

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
    fn packed_batch_restores_a_missing_derived_loose_object() {
        let (_root, store) = store();
        let object = store
            .put_batch(vec![("test/packed/1".into(), b"durable payload".to_vec())])
            .unwrap()
            .remove(0);
        let loose = store.object_path(&object.digest).unwrap();
        fs::remove_file(&loose).unwrap();
        assert_eq!(
            store.read(&object, 1024).unwrap().bytes(),
            b"durable payload"
        );
        assert!(loose.is_file());
    }

    #[test]
    fn packed_batch_never_returns_tampered_data() {
        let (_root, store) = store();
        let object = store
            .put_batch(vec![("test/packed/1".into(), b"trusted payload".to_vec())])
            .unwrap()
            .remove(0);
        fs::remove_file(store.object_path(&object.digest).unwrap()).unwrap();
        let pack = fs::read_dir(&store.packs)
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
        let (_root, store) = store();
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
}
