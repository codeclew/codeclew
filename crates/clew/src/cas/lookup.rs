//! A derived, paged lookup for a verified catalog snapshot. The JSON snapshot
//! remains authoritative for publication, recovery and collection. Admission
//! receipts bind both files to fd metadata (as pack verification receipts do);
//! each page is also hashed before its entries can supply an object location.

use super::{
    CAS_OBJECT_SCHEMA, CasObject, CatalogHead, CatalogSnapshot, ClewError, ManagedDirectory,
    PackLocation, canonical, corrupt, digest_component, internal, validate_object_schema,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, VecDeque};
use std::ffi::OsStr;
use std::fs::File;
use std::os::unix::fs::{FileExt, MetadataExt};
use std::sync::{Arc, Mutex};

const SCHEMA: &str = "codeclew-catalog-lookup/1.0";
const RECEIPT_SCHEMA: &str = "codeclew-catalog-lookup-receipt/1.0";
const MIN_OBJECTS: usize = 1024;
const ROW_BYTES: usize = 56;
const PAGE_ROWS: usize = 256;
const CACHE_PAGES: usize = 32;
const MAX_MANIFEST_BYTES: usize = 16 * 1024 * 1024;
const MAX_DATA_BYTES: u64 = 256 * 1024 * 1024;
const SUFFIXES: [&str; 3] = [".lookup-v1", ".lookup-v1.json", ".lookup-v1.receipt"];

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Manifest {
    schema: String,
    snapshot_digest: String,
    sequence: u64,
    last_record_digest: Option<String>,
    object_count: u64,
    data_bytes: u64,
    packs: Vec<Pack>,
    object_schemas: Vec<String>,
    pages: Vec<Page>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Pack {
    data_name: String,
    data_size: u64,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Page {
    first: String,
    last: String,
    digest: String,
    offset: u64,
    rows: u32,
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Fingerprint {
    device: u64,
    inode: u64,
    bytes: u64,
    modified_seconds: i64,
    modified_nanoseconds: i64,
    changed_seconds: i64,
    changed_nanoseconds: i64,
}

impl Fingerprint {
    fn read(file: &File) -> Result<Self, ClewError> {
        let metadata = file.metadata().map_err(internal)?;
        Ok(Self {
            device: metadata.dev(),
            inode: metadata.ino(),
            bytes: metadata.len(),
            modified_seconds: metadata.mtime(),
            modified_nanoseconds: metadata.mtime_nsec(),
            changed_seconds: metadata.ctime(),
            changed_nanoseconds: metadata.ctime_nsec(),
        })
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Receipt {
    schema: String,
    snapshot_digest: String,
    snapshot: Fingerprint,
    manifest_digest: String,
    data: Fingerprint,
}

#[derive(Debug, Clone, Copy)]
struct Row {
    digest: [u8; 32],
    pack: u32,
    schema: u32,
    size: u64,
    offset: u64,
}

#[derive(Debug)]
pub(super) struct Lookup {
    manifest: Manifest,
    file: File,
    cache: Mutex<VecDeque<(usize, Arc<Vec<Row>>)>>,
    #[cfg(test)]
    pages_read: std::sync::atomic::AtomicUsize,
}

impl Lookup {
    pub(super) fn contains_pack(&self, name: &str) -> bool {
        self.manifest
            .packs
            .binary_search_by(|pack| pack.data_name.as_str().cmp(name))
            .is_ok()
    }

    pub(super) fn find(&self, digest: &str) -> Result<Option<PackLocation>, ClewError> {
        let wanted = binary_digest(digest)?;
        let page = self
            .manifest
            .pages
            .partition_point(|page| page.last.as_str() < digest);
        let Some(reference) = self.manifest.pages.get(page) else {
            return Ok(None);
        };
        if digest < reference.first.as_str() {
            return Ok(None);
        }
        let rows = self.read_page(page)?;
        let Ok(position) = rows.binary_search_by_key(&wanted, |row| row.digest) else {
            return Ok(None);
        };
        let row = rows[position];
        Ok(Some(PackLocation {
            data_name: self.manifest.packs[row.pack as usize].data_name.clone(),
            entry: super::PackEntry {
                object: CasObject {
                    schema: CAS_OBJECT_SCHEMA.into(),
                    object_schema: self.manifest.object_schemas[row.schema as usize].clone(),
                    digest: digest.into(),
                    size: row.size,
                },
                offset: row.offset,
            },
        }))
    }

    fn read_page(&self, index: usize) -> Result<Arc<Vec<Row>>, ClewError> {
        let mut cache = self
            .cache
            .lock()
            .map_err(|_| internal("catalog lookup cache lock is poisoned"))?;
        if let Some((_, rows)) = cache.iter().find(|(page, _)| *page == index) {
            return Ok(Arc::clone(rows));
        }
        let page = &self.manifest.pages[index];
        let mut bytes = vec![0; page.rows as usize * ROW_BYTES];
        self.file
            .read_exact_at(&mut bytes, page.offset)
            .map_err(|_| corrupt("catalog lookup page is truncated"))?;
        if canonical::hash_bytes(&bytes) != page.digest {
            return Err(corrupt("catalog lookup page digest mismatch"));
        }
        let mut rows = Vec::<Row>::with_capacity(page.rows as usize);
        for bytes in bytes.chunks_exact(ROW_BYTES) {
            let row = Row {
                digest: bytes[..32].try_into().expect("fixed digest"),
                pack: u32::from_le_bytes(bytes[32..36].try_into().expect("fixed pack")),
                schema: u32::from_le_bytes(bytes[36..40].try_into().expect("fixed schema")),
                size: u64::from_le_bytes(bytes[40..48].try_into().expect("fixed size")),
                offset: u64::from_le_bytes(bytes[48..56].try_into().expect("fixed offset")),
            };
            let pack = self
                .manifest
                .packs
                .get(row.pack as usize)
                .ok_or_else(|| corrupt("catalog lookup pack ordinal is invalid"))?;
            if row.schema as usize >= self.manifest.object_schemas.len()
                || row
                    .offset
                    .checked_add(row.size)
                    .is_none_or(|end| end > pack.data_size)
                || rows
                    .last()
                    .is_some_and(|previous| previous.digest >= row.digest)
            {
                return Err(corrupt("catalog lookup row authority is invalid"));
            }
            rows.push(row);
        }
        if rows.first().map(|row| row.digest) != Some(binary_digest(&page.first)?)
            || rows.last().map(|row| row.digest) != Some(binary_digest(&page.last)?)
        {
            return Err(corrupt("catalog lookup page range is invalid"));
        }
        #[cfg(test)]
        self.pages_read
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let rows = Arc::new(rows);
        if cache.len() == CACHE_PAGES {
            cache.pop_front();
        }
        cache.push_back((index, Arc::clone(&rows)));
        Ok(rows)
    }
}

pub(super) fn open(
    directory: &ManagedDirectory,
    head: &CatalogHead,
) -> Result<Option<Lookup>, ClewError> {
    let names = names(&head.snapshot_digest)?;
    if !directory.file_exists(OsStr::new(&names[2]))? {
        return Ok(None);
    }
    let receipt_bytes = directory.read_file(OsStr::new(&names[2]), 4096)?;
    let receipt: Receipt = serde_json::from_slice(&receipt_bytes)
        .map_err(|_| corrupt("catalog lookup receipt is invalid"))?;
    if receipt.schema != RECEIPT_SCHEMA
        || receipt.snapshot_digest != head.snapshot_digest
        || canonical::bytes(&receipt).map_err(internal)? != receipt_bytes
    {
        return Err(corrupt("catalog lookup receipt authority mismatch"));
    }
    let snapshot = directory.open_file(OsStr::new(&head.snapshot_name))?;
    if Fingerprint::read(&snapshot)? != receipt.snapshot {
        // Recheck authoritative JSON after any source identity/metadata change.
        return Ok(None);
    }
    let bytes = directory.read_file(OsStr::new(&names[1]), MAX_MANIFEST_BYTES)?;
    if canonical::hash_bytes(&bytes) != receipt.manifest_digest {
        return Err(corrupt("catalog lookup manifest digest mismatch"));
    }
    let manifest: Manifest = serde_json::from_slice(&bytes)
        .map_err(|_| corrupt("catalog lookup manifest is invalid"))?;
    if canonical::bytes(&manifest).map_err(internal)? != bytes {
        return Err(corrupt("catalog lookup manifest is not canonical"));
    }
    validate_manifest(&manifest, head)?;
    let file = directory.open_file(OsStr::new(&names[0]))?;
    if Fingerprint::read(&file)? != receipt.data || receipt.data.bytes != manifest.data_bytes {
        return Err(corrupt("catalog lookup data identity changed"));
    }
    Ok(Some(Lookup {
        manifest,
        file,
        cache: Mutex::new(VecDeque::new()),
        #[cfg(test)]
        pages_read: std::sync::atomic::AtomicUsize::new(0),
    }))
}

fn validate_manifest(manifest: &Manifest, head: &CatalogHead) -> Result<(), ClewError> {
    if manifest.schema != SCHEMA
        || manifest.snapshot_digest != head.snapshot_digest
        || manifest.sequence != head.snapshot_sequence
        || manifest.last_record_digest != head.last_record_digest
        || manifest.object_count == 0
        || manifest.data_bytes > MAX_DATA_BYTES
        || manifest.object_count.checked_mul(ROW_BYTES as u64) != Some(manifest.data_bytes)
        || manifest.packs.is_empty()
        || manifest.object_schemas.is_empty()
        || !manifest
            .packs
            .windows(2)
            .all(|pair| pair[0].data_name < pair[1].data_name)
        || !manifest
            .object_schemas
            .windows(2)
            .all(|pair| pair[0] < pair[1])
    {
        return Err(corrupt("catalog lookup manifest authority mismatch"));
    }
    for pack in &manifest.packs {
        let component = pack
            .data_name
            .strip_suffix(".pack")
            .ok_or_else(|| corrupt("catalog lookup pack name is invalid"))?;
        digest_component(&format!("sha256:{component}"))?;
    }
    for schema in &manifest.object_schemas {
        validate_object_schema(schema)?;
    }
    let mut count = 0u64;
    let mut previous: Option<&str> = None;
    for (index, page) in manifest.pages.iter().enumerate() {
        binary_digest(&page.first)?;
        binary_digest(&page.last)?;
        binary_digest(&page.digest)?;
        if page.rows == 0
            || page.rows as usize > PAGE_ROWS
            || (index + 1 < manifest.pages.len() && page.rows as usize != PAGE_ROWS)
            || page.first > page.last
            || previous.is_some_and(|last| last >= page.first.as_str())
            || count.checked_mul(ROW_BYTES as u64) != Some(page.offset)
        {
            return Err(corrupt("catalog lookup page directory is invalid"));
        }
        count += page.rows as u64;
        previous = Some(&page.last);
    }
    if count != manifest.object_count {
        return Err(corrupt("catalog lookup page count is invalid"));
    }
    Ok(())
}

/// Called under the catalog publication lock after full snapshot verification.
/// The receipt is the commit point; incomplete sidecars are never admitted.
pub(super) fn publish(
    directory: &ManagedDirectory,
    snapshot: &CatalogSnapshot,
    digest: &str,
) -> Result<(), ClewError> {
    let count = snapshot
        .packs
        .iter()
        .map(|pack| pack.manifest.objects.len())
        .sum::<usize>();
    if count < MIN_OBJECTS {
        return Ok(());
    }
    if count as u64 > MAX_DATA_BYTES / ROW_BYTES as u64 {
        return Ok(());
    }
    let names = names(digest)?;
    let snapshot_file = directory.open_file(OsStr::new(&format!(
        "{}.snapshot",
        digest_component(digest)?
    )))?;
    let fingerprint = Fingerprint::read(&snapshot_file)?;
    let schemas = snapshot
        .packs
        .iter()
        .flat_map(|pack| &pack.manifest.objects)
        .map(|entry| entry.object.object_schema.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let mut rows = Vec::with_capacity(count);
    for (pack_index, pack) in snapshot.packs.iter().enumerate() {
        for entry in &pack.manifest.objects {
            rows.push(Row {
                digest: binary_digest(&entry.object.digest)?,
                pack: u32::try_from(pack_index).map_err(internal)?,
                schema: schemas
                    .binary_search(&entry.object.object_schema)
                    .expect("collected schema") as u32,
                size: entry.object.size,
                offset: entry.offset,
            });
        }
    }
    rows.sort_unstable_by_key(|row| (row.digest, row.pack));
    for pair in rows.windows(2) {
        if pair[0].digest == pair[1].digest
            && (pair[0].schema != pair[1].schema || pair[0].size != pair[1].size)
        {
            return Err(corrupt("catalog lookup contains a digest collision"));
        }
    }
    rows.dedup_by_key(|row| row.digest);
    let mut data = Vec::with_capacity(rows.len() * ROW_BYTES);
    let mut pages = Vec::new();
    for page in rows.chunks(PAGE_ROWS) {
        let offset = data.len();
        for row in page {
            data.extend_from_slice(&row.digest);
            data.extend_from_slice(&row.pack.to_le_bytes());
            data.extend_from_slice(&row.schema.to_le_bytes());
            data.extend_from_slice(&row.size.to_le_bytes());
            data.extend_from_slice(&row.offset.to_le_bytes());
        }
        pages.push(Page {
            first: format!(
                "sha256:{}",
                hex::encode(page.first().expect("nonempty page").digest)
            ),
            last: format!(
                "sha256:{}",
                hex::encode(page.last().expect("nonempty page").digest)
            ),
            digest: canonical::hash_bytes(&data[offset..]),
            offset: offset as u64,
            rows: page.len() as u32,
        });
    }
    let manifest = Manifest {
        schema: SCHEMA.into(),
        snapshot_digest: digest.into(),
        sequence: snapshot.sequence,
        last_record_digest: snapshot.last_record_digest.clone(),
        object_count: rows.len() as u64,
        data_bytes: data.len() as u64,
        packs: snapshot
            .packs
            .iter()
            .map(|pack| Pack {
                data_name: pack.data_name.clone(),
                data_size: pack.manifest.data_size,
            })
            .collect(),
        object_schemas: schemas,
        pages,
    };
    let bytes = canonical::bytes(&manifest).map_err(internal)?;
    if bytes.len() > MAX_MANIFEST_BYTES {
        return Ok(());
    }
    // Remove the commit marker before replacing any sidecar. A crash during
    // replacement must select JSON recovery, not an old receipt with new data.
    if directory.file_exists(OsStr::new(&names[2]))? {
        directory.remove_file(OsStr::new(&names[2]))?;
    }
    directory.atomic_write(OsStr::new(&names[0]), &data)?;
    directory.atomic_write(OsStr::new(&names[1]), &bytes)?;
    let receipt = Receipt {
        schema: RECEIPT_SCHEMA.into(),
        snapshot_digest: digest.into(),
        snapshot: fingerprint,
        manifest_digest: canonical::hash_bytes(&bytes),
        data: Fingerprint::read(&directory.open_file(OsStr::new(&names[0]))?)?,
    };
    directory.atomic_write(
        OsStr::new(&names[2]),
        &canonical::bytes(&receipt).map_err(internal)?,
    )
}

pub(super) fn obsolete(name: &OsStr, current_snapshot: &str) -> bool {
    let Some(name) = name.to_str() else {
        return false;
    };
    SUFFIXES.iter().any(|suffix| {
        name.strip_suffix(suffix).is_some_and(|component| {
            component.len() == 64
                && component
                    .bytes()
                    .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
                && format!("{component}.snapshot") != current_snapshot
        })
    })
}

fn names(digest: &str) -> Result<[String; 3], ClewError> {
    let component = digest_component(digest)?;
    Ok(SUFFIXES.map(|suffix| format!("{component}{suffix}")))
}

fn binary_digest(digest: &str) -> Result<[u8; 32], ClewError> {
    let mut bytes = [0; 32];
    hex::decode_to_slice(digest_component(digest)?, &mut bytes)
        .map_err(|_| corrupt("catalog lookup digest is invalid"))?;
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cas::{CasStore, ErrorCode, garbage_collect_storage};
    use crate::state::StateAuthority;
    use std::fs;
    use std::path::Path;
    use std::sync::atomic::Ordering;

    fn fixture() -> (tempfile::TempDir, StateAuthority, CasStore, Vec<CasObject>) {
        fixture_with_count(2048)
    }

    fn fixture_with_count(
        count: usize,
    ) -> (tempfile::TempDir, StateAuthority, CasStore, Vec<CasObject>) {
        let root = tempfile::tempdir().unwrap();
        let authority = StateAuthority::open(root.path().join("v2")).unwrap();
        let store = CasStore::open(&authority).unwrap();
        let objects = store
            .put_batch(
                (0..count)
                    .map(|i| ("test/lookup/1".into(), format!("item-{i}").into_bytes()))
                    .collect(),
            )
            .unwrap();
        let lock = store.batch_lock().unwrap();
        store.maybe_snapshot_catalog_locked(true).unwrap();
        drop(lock);
        (root, authority, store, objects)
    }

    fn reopen(authority: &StateAuthority) -> CasStore {
        CasStore::open_with_catalog(authority, false).unwrap()
    }

    fn lookup(store: &CasStore) -> Arc<Lookup> {
        Arc::clone(
            store
                .pack_catalog
                .read()
                .unwrap()
                .lookup
                .as_ref()
                .expect("selective catalog"),
        )
    }

    #[test]
    fn page_cache_evicts_oldest_page_at_its_bound() {
        let (_root, authority, _writer, _objects) =
            fixture_with_count((CACHE_PAGES + 1) * PAGE_ROWS);
        let reader = reopen(&authority);
        let index = lookup(&reader);
        for page in &index.manifest.pages {
            assert!(index.find(&page.first).unwrap().is_some());
        }
        assert_eq!(index.cache.lock().unwrap().len(), CACHE_PAGES);
        assert_eq!(index.pages_read.load(Ordering::Relaxed), CACHE_PAGES + 1);
        assert!(
            index
                .find(&index.manifest.pages[1].first)
                .unwrap()
                .is_some()
        );
        assert_eq!(index.pages_read.load(Ordering::Relaxed), CACHE_PAGES + 1);
        assert!(
            index
                .find(&index.manifest.pages[0].first)
                .unwrap()
                .is_some()
        );
        assert_eq!(index.pages_read.load(Ordering::Relaxed), CACHE_PAGES + 2);
        assert_eq!(index.cache.lock().unwrap().len(), CACHE_PAGES);
    }

    #[test]
    fn context_reads_only_selected_pages_and_reuses_a_bounded_cache() {
        let (_root, authority, _writer, objects) = fixture();
        let reader = reopen(&authority);
        let index = lookup(&reader);
        {
            let state = reader.pack_catalog.read().unwrap();
            assert!(state.packs.is_empty());
            assert!(state.locations.is_empty());
        }
        assert_eq!(index.pages_read.load(Ordering::Relaxed), 0);
        for i in [0, 1024, 2047] {
            assert_eq!(
                reader.read(&objects[i], 64).unwrap().bytes(),
                format!("item-{i}").as_bytes()
            );
        }
        let count = index.pages_read.load(Ordering::Relaxed);
        assert!(count <= 3 && count < index.manifest.pages.len());
        for i in [0, 1024, 2047] {
            reader.read(&objects[i], 64).unwrap();
        }
        assert_eq!(index.pages_read.load(Ordering::Relaxed), count);
        assert_eq!(
            reader.read(&objects[0], 1).unwrap_err().code,
            ErrorCode::ResourceLimit
        );
        let mut wrong = objects[0].clone();
        wrong.size += 1;
        assert_eq!(
            reader.read(&wrong, 64).unwrap_err().code,
            ErrorCode::StateCorrupt
        );
        assert_eq!(index.pages_read.load(Ordering::Relaxed), count);
    }

    #[test]
    fn journal_overlay_checks_visit_each_base_page_at_most_once() {
        let (_root, authority, writer, _objects) =
            fixture_with_count((CACHE_PAGES + 1) * PAGE_ROWS);
        let added = writer
            .put_batch(
                (0..2048)
                    .map(|i| ("test/lookup/1".into(), format!("added-{i}").into_bytes()))
                    .collect(),
            )
            .unwrap();
        let reader = reopen(&authority);
        let index = lookup(&reader);
        assert_eq!(
            reader.pack_catalog.read().unwrap().locations.len(),
            added.len()
        );
        assert_eq!(
            index.pages_read.load(Ordering::Relaxed),
            index.manifest.pages.len()
        );
        assert_eq!(reader.read(&added[0], 64).unwrap().bytes(), b"added-0");
    }

    #[test]
    fn changed_snapshot_is_reverified_and_missing_receipt_recovers_old_format() {
        let (_root, authority, writer, objects) = fixture();
        let head = writer.read_catalog_head().unwrap();
        let snapshot_path = writer.catalog_snapshots.path().join(&head.snapshot_name);
        let original = fs::read(&snapshot_path).unwrap();
        // Identical bytes with a new inode invalidate admission but remain valid.
        writer
            .catalog_snapshots
            .atomic_write(OsStr::new(&head.snapshot_name), &original)
            .unwrap();
        let recovered = reopen(&authority);
        assert!(recovered.pack_catalog.read().unwrap().lookup.is_none());
        assert_eq!(recovered.read(&objects[0], 64).unwrap().bytes(), b"item-0");
        lookup(&reopen(&authority));

        let names = names(&head.snapshot_digest).unwrap();
        writer
            .catalog_snapshots
            .remove_file(OsStr::new(&names[2]))
            .unwrap();
        // Incomplete derived files without their commit marker are not authority.
        fs::write(writer.catalog_snapshots.path().join(&names[0]), b"partial").unwrap();
        fs::write(writer.catalog_snapshots.path().join(&names[1]), b"partial").unwrap();
        let recovered = reopen(&authority);
        assert!(recovered.pack_catalog.read().unwrap().lookup.is_none());
        assert_eq!(recovered.read(&objects[1], 64).unwrap().bytes(), b"item-1");
        lookup(&reopen(&authority));

        let mut changed = original;
        changed[0] = b'[';
        fs::write(&snapshot_path, changed).unwrap();
        assert_eq!(
            CasStore::open_with_catalog(&authority, false)
                .unwrap_err()
                .code,
            ErrorCode::StateCorrupt
        );
    }

    #[test]
    fn changed_index_fails_admission_and_page_hash_catches_post_open_changes() {
        let (_root, authority, writer, objects) = fixture();
        let reader = reopen(&authority);
        let index = lookup(&reader);
        let head = writer.read_catalog_head().unwrap();
        let names = names(&head.snapshot_digest).unwrap();
        let path = writer.catalog_snapshots.path().join(&names[0]);
        let mut data = fs::read(&path).unwrap();
        data[40] ^= 1;
        fs::write(&path, data).unwrap();
        assert_eq!(
            index.find(&index.manifest.pages[0].first).unwrap_err().code,
            ErrorCode::StateCorrupt
        );
        assert_eq!(
            CasStore::open_with_catalog(&authority, false)
                .unwrap_err()
                .code,
            ErrorCode::StateCorrupt
        );
        assert!(
            objects
                .iter()
                .any(|object| object.digest == index.manifest.pages[0].first)
        );
    }

    #[test]
    fn malformed_binary_ordinals_fail_even_with_rehashed_derived_metadata() {
        let (_root, authority, writer, _objects) = fixture();
        let head = writer.read_catalog_head().unwrap();
        let names = names(&head.snapshot_digest).unwrap();
        let directory = &writer.catalog_snapshots;
        let mut manifest: Manifest = serde_json::from_slice(
            &directory
                .read_file(OsStr::new(&names[1]), MAX_MANIFEST_BYTES)
                .unwrap(),
        )
        .unwrap();
        let mut receipt: Receipt =
            serde_json::from_slice(&directory.read_file(OsStr::new(&names[2]), 4096).unwrap())
                .unwrap();
        let mut data = fs::read(directory.path().join(&names[0])).unwrap();
        data[32..36].copy_from_slice(&u32::MAX.to_le_bytes());
        manifest.pages[0].digest = canonical::hash_bytes(&data[..PAGE_ROWS * ROW_BYTES]);
        let bytes = canonical::bytes(&manifest).unwrap();
        directory
            .atomic_write(OsStr::new(&names[0]), &data)
            .unwrap();
        directory
            .atomic_write(OsStr::new(&names[1]), &bytes)
            .unwrap();
        receipt.data =
            Fingerprint::read(&directory.open_file(OsStr::new(&names[0])).unwrap()).unwrap();
        receipt.manifest_digest = canonical::hash_bytes(&bytes);
        directory
            .atomic_write(OsStr::new(&names[2]), &canonical::bytes(&receipt).unwrap())
            .unwrap();
        let reader = reopen(&authority);
        assert_eq!(
            lookup(&reader)
                .find(&manifest.pages[0].first)
                .unwrap_err()
                .code,
            ErrorCode::StateCorrupt
        );
    }

    #[test]
    fn journal_additions_refresh_readers_and_corrupt_tail_fails_closed() {
        let (_root, authority, writer, objects) = fixture();
        let reader = reopen(&authority);
        let publisher = reopen(&authority);
        let added = publisher
            .put_batch(vec![("test/lookup/1".into(), b"added".to_vec())])
            .unwrap()
            .remove(0);
        assert!(publisher.pack_catalog.read().unwrap().lookup.is_none());
        // An independent reader recovers a new location on a digest miss.
        assert_eq!(reader.read(&added, 64).unwrap().bytes(), b"added");
        assert_eq!(reader.read(&objects[0], 64).unwrap().bytes(), b"item-0");
        let fresh = reopen(&authority);
        lookup(&fresh);
        assert_eq!(fresh.read(&added, 64).unwrap().bytes(), b"added");
        assert_eq!(fresh.pack_catalog.read().unwrap().locations.len(), 1);
        let record = writer
            .catalog_records
            .entries()
            .unwrap()
            .into_iter()
            .find(|name| Path::new(name).extension() == Some(OsStr::new("record")))
            .unwrap();
        fs::write(writer.catalog_records.path().join(record), b"corrupt").unwrap();
        assert_eq!(
            CasStore::open_with_catalog(&authority, false)
                .unwrap_err()
                .code,
            ErrorCode::StateCorrupt
        );
    }

    #[test]
    fn pinned_reader_survives_snapshot_rotation_and_gc_keeps_live_pack() {
        let (_root, authority, writer, objects) = fixture();
        let reader = reopen(&authority);
        let previous = writer.read_catalog_head().unwrap();
        writer
            .put_batch(vec![("test/lookup/1".into(), b"dead".to_vec())])
            .unwrap();
        {
            let _lock = writer.batch_lock().unwrap();
            writer.maybe_snapshot_catalog_locked(true).unwrap();
        }
        for name in names(&previous.snapshot_digest).unwrap() {
            assert!(
                !writer
                    .catalog_snapshots
                    .file_exists(OsStr::new(&name))
                    .unwrap()
            );
        }
        assert_eq!(reader.read(&objects[0], 64).unwrap().bytes(), b"item-0");
        authority
            .directory(Path::new("repos"))
            .unwrap()
            .atomic_write(
                OsStr::new("retained.json"),
                &canonical::bytes(&serde_json::json!({"object":objects[0]})).unwrap(),
            )
            .unwrap();
        drop(reader);
        drop(writer);
        let report = garbage_collect_storage(&authority).unwrap();
        assert_eq!(report.reclaimable_packs, 1);
        let reader = reopen(&authority);
        lookup(&reader);
        assert_eq!(reader.read(&objects[0], 64).unwrap().bytes(), b"item-0");
    }
}
