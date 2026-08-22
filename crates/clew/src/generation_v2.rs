use crate::adapter_v2::{AnalysisAttemptComplete, CapabilityUri, FactRecord};
use crate::canonical;
use crate::cas::{CasObject, CasStore};
use crate::derived_manifest::DERIVED_MANIFEST_SCHEMA;
use crate::error::{ClewError, ErrorCode};
use crate::state::{StateAuthority, create_private_directory};
use serde::{Deserialize, Serialize};
use std::cmp::{Ordering, Reverse};
use std::collections::{BTreeSet, BinaryHeap};
use std::fs::{self, File, OpenOptions};
use std::io::{BufReader, BufWriter, Read, Write};
use std::path::PathBuf;

#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;

pub const FACT_SHARD_SCHEMA: &str = "codeclew-canonical-fact-shard/2.0";
pub const GENERATION_SCHEMA: &str = "codeclew-generation-manifest/2.0";
pub const MAX_SHARD_BYTES: usize = 8 * 1024 * 1024;
const MAX_FACT_BYTES: usize = 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AttemptAuthority {
    pub compilation_id: String,
    pub capability: CapabilityUri,
    pub completion: AnalysisAttemptComplete,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GenerationManifest {
    pub schema: String,
    pub generation_id: String,
    pub derived_input_manifest: CasObject,
    pub parent_generation: Option<CasObject>,
    pub generation_kind: GenerationKind,
    pub attempts: Vec<AttemptAuthority>,
    pub shards: Vec<CasObject>,
    pub fact_count: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum GenerationKind {
    Full,
    Delta,
}

impl GenerationManifest {
    pub fn verify_manifest(&self, store: &CasStore) -> Result<(), ClewError> {
        let mut unsigned = self.clone();
        unsigned.generation_id.clear();
        if self.schema != GENERATION_SCHEMA
            || self.derived_input_manifest.object_schema != DERIVED_MANIFEST_SCHEMA
            || self.generation_id != canonical::hash(&unsigned).map_err(internal)?
        {
            return Err(corrupt("generation manifest identity is invalid"));
        }
        if let Some(parent) = &self.parent_generation {
            if self.generation_kind != GenerationKind::Delta
                || parent.object_schema != GENERATION_SCHEMA
            {
                return Err(corrupt("generation parent authority is invalid"));
            }
            store
                .read(
                    parent,
                    usize::try_from(parent.size).map_err(|_| {
                        ClewError::new(
                            ErrorCode::ResourceLimit,
                            "parent generation exceeds host size",
                        )
                    })?,
                )
                .map_err(|_| corrupt("parent generation is unavailable"))?;
        } else if self.generation_kind != GenerationKind::Full {
            return Err(corrupt("delta generation has no parent"));
        }
        verify_attempts(store, &self.attempts)?;
        let derived_limit = usize::try_from(self.derived_input_manifest.size).map_err(|_| {
            ClewError::new(
                ErrorCode::ResourceLimit,
                "derived manifest exceeds host size",
            )
        })?;
        store
            .read(&self.derived_input_manifest, derived_limit)
            .map_err(|_| corrupt("generation derived manifest is unavailable"))?;
        if self.shards.is_empty()
            || self.fact_count == 0
            || self.shards.iter().any(|reference| {
                reference.object_schema != FACT_SHARD_SCHEMA
                    || reference.size > MAX_SHARD_BYTES as u64
            })
        {
            return Err(corrupt("generation shard references are invalid"));
        }
        Ok(())
    }

    pub fn verify(&self, store: &CasStore) -> Result<(), ClewError> {
        self.verify_manifest(store)?;
        let mut expected_sequence = 0u32;
        let mut observed_count = 0u64;
        let mut last_key = None::<String>;
        for reference in &self.shards {
            let lease = store.read(reference, MAX_SHARD_BYTES)?;
            let shard: CanonicalFactShard = serde_json::from_slice(lease.bytes())
                .map_err(|_| corrupt("generation shard is not a closed canonical object"))?;
            if canonical::bytes(&shard).map_err(internal)? != lease.bytes()
                || shard.schema != FACT_SHARD_SCHEMA
                || shard.sequence != expected_sequence
                || shard.facts.is_empty()
                || shard.first_key != shard.facts.first().expect("non-empty shard").fact_key
                || shard.last_key != shard.facts.last().expect("non-empty shard").fact_key
            {
                return Err(corrupt("generation shard authority is invalid"));
            }
            for fact in &shard.facts {
                fact.validate()?;
                if last_key
                    .as_deref()
                    .is_some_and(|previous| previous >= fact.fact_key.as_str())
                {
                    return Err(corrupt("generation fact order is invalid"));
                }
                last_key = Some(fact.fact_key.clone());
                observed_count += 1;
            }
            expected_sequence = expected_sequence
                .checked_add(1)
                .ok_or_else(|| corrupt("generation shard sequence overflow"))?;
        }
        if observed_count != self.fact_count {
            return Err(corrupt("generation fact count is incomplete"));
        }
        Ok(())
    }

    pub fn visit_facts(
        &self,
        store: &CasStore,
        mut visitor: impl FnMut(&FactRecord) -> Result<(), ClewError>,
    ) -> Result<(), ClewError> {
        self.verify(store)?;
        for reference in &self.shards {
            let lease = store.read(reference, MAX_SHARD_BYTES)?;
            let shard: CanonicalFactShard = serde_json::from_slice(lease.bytes())
                .map_err(|_| corrupt("generation shard is not a closed canonical object"))?;
            for fact in &shard.facts {
                visitor(fact)?;
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CanonicalFactShard {
    schema: String,
    sequence: u32,
    first_key: String,
    last_key: String,
    facts: Vec<FactRecord>,
}

pub struct FactRunWriter {
    path: PathBuf,
    writer: BufWriter<File>,
    last_key: Option<String>,
    count: u64,
    delete_on_drop: bool,
}

pub struct FactRun {
    path: PathBuf,
    count: u64,
}

impl FactRunWriter {
    pub fn create(state: &StateAuthority) -> Result<Self, ClewError> {
        let directory = state.attempts_root().join("fact-runs");
        create_private_directory(&directory)?;
        let path = directory.join(format!("run-{}", uuid::Uuid::new_v4()));
        let mut options = OpenOptions::new();
        options.create_new(true).write(true);
        #[cfg(unix)]
        options.mode(0o600);
        let file = options.open(&path).map_err(io_error)?;
        Ok(Self {
            path,
            writer: BufWriter::new(file),
            last_key: None,
            count: 0,
            delete_on_drop: true,
        })
    }

    pub fn push(&mut self, fact: &FactRecord) -> Result<(), ClewError> {
        fact.validate()?;
        if self
            .last_key
            .as_deref()
            .is_some_and(|previous| previous >= fact.fact_key.as_str())
        {
            return Err(protocol("fact run is not strictly sorted"));
        }
        let bytes = canonical::bytes(fact).map_err(internal)?;
        if bytes.len() > MAX_FACT_BYTES || bytes.len() > u32::MAX as usize {
            return Err(ClewError::new(
                ErrorCode::ResourceLimit,
                "fact exceeds the run record limit",
            ));
        }
        self.writer
            .write_all(&(bytes.len() as u32).to_be_bytes())
            .and_then(|_| self.writer.write_all(&bytes))
            .map_err(io_error)?;
        self.last_key = Some(fact.fact_key.clone());
        self.count += 1;
        Ok(())
    }

    pub fn finish(mut self) -> Result<FactRun, ClewError> {
        self.writer.flush().map_err(io_error)?;
        self.writer.get_ref().sync_all().map_err(io_error)?;
        self.delete_on_drop = false;
        Ok(FactRun {
            path: self.path.clone(),
            count: self.count,
        })
    }
}

impl Drop for FactRunWriter {
    fn drop(&mut self) {
        if self.delete_on_drop {
            let _ = fs::remove_file(&self.path);
        }
    }
}

impl Drop for FactRun {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

pub fn finalize_generation(
    store: &CasStore,
    derived_input_manifest: CasObject,
    attempts: Vec<AttemptAuthority>,
    runs: Vec<FactRun>,
) -> Result<(GenerationManifest, CasObject), ClewError> {
    finalize_generation_with_parent(store, derived_input_manifest, attempts, runs, None)
}

pub fn finalize_generation_with_parent(
    store: &CasStore,
    derived_input_manifest: CasObject,
    attempts: Vec<AttemptAuthority>,
    runs: Vec<FactRun>,
    parent_generation: Option<CasObject>,
) -> Result<(GenerationManifest, CasObject), ClewError> {
    finalize_with_limit(
        store,
        derived_input_manifest,
        attempts,
        runs,
        parent_generation,
        MAX_SHARD_BYTES,
    )
}

fn finalize_with_limit(
    store: &CasStore,
    derived_input_manifest: CasObject,
    mut attempts: Vec<AttemptAuthority>,
    runs: Vec<FactRun>,
    parent_generation: Option<CasObject>,
    shard_limit: usize,
) -> Result<(GenerationManifest, CasObject), ClewError> {
    if derived_input_manifest.object_schema != DERIVED_MANIFEST_SCHEMA {
        return Err(invalid("generation requires a derived input manifest v2"));
    }
    if let Some(parent) = &parent_generation {
        if parent.object_schema != GENERATION_SCHEMA {
            return Err(invalid("generation parent has the wrong schema"));
        }
        store.read(
            parent,
            usize::try_from(parent.size).map_err(|_| {
                ClewError::new(
                    ErrorCode::ResourceLimit,
                    "parent generation exceeds host size",
                )
            })?,
        )?;
    }
    if runs.is_empty() || shard_limit == 0 || shard_limit > MAX_SHARD_BYTES {
        return Err(invalid("generation run set or shard limit is invalid"));
    }
    attempts.sort_by(|left, right| {
        (&left.compilation_id, left.capability.as_str())
            .cmp(&(&right.compilation_id, right.capability.as_str()))
    });
    if attempts.is_empty()
        || !attempts.windows(2).all(|pair| {
            (&pair[0].compilation_id, pair[0].capability.as_str())
                < (&pair[1].compilation_id, pair[1].capability.as_str())
        })
    {
        return Err(invalid(
            "generation attempt authority is empty or duplicated",
        ));
    }
    verify_attempts(store, &attempts)?;
    let expected_count = runs.iter().try_fold(0u64, |total, run| {
        total
            .checked_add(run.count)
            .ok_or_else(|| ClewError::new(ErrorCode::ResourceLimit, "fact count overflow"))
    })?;
    let completed_count = attempts.iter().try_fold(0u64, |total, attempt| {
        total
            .checked_add(attempt.completion.fact_count)
            .ok_or_else(|| {
                ClewError::new(ErrorCode::ResourceLimit, "completion fact count overflow")
            })
    })?;
    if completed_count != expected_count {
        return Err(protocol(
            "attempt completion count differs from sealed fact runs",
        ));
    }
    let mut readers = runs
        .iter()
        .map(RunReader::open)
        .collect::<Result<Vec<_>, ClewError>>()?;
    let mut heap = BinaryHeap::<Reverse<HeapFact>>::new();
    for (index, reader) in readers.iter_mut().enumerate() {
        if let Some(fact) = reader.next()? {
            heap.push(Reverse(HeapFact { run: index, fact }));
        }
    }
    let mut sequence = 0u32;
    let mut current = Vec::<EncodedFact>::new();
    let mut current_records_bytes = 0usize;
    let mut shards = Vec::new();
    let mut fact_count = 0u64;
    let mut last_key = None::<String>;
    while let Some(Reverse(item)) = heap.pop() {
        if last_key
            .as_deref()
            .is_some_and(|previous| previous >= item.fact.record.fact_key.as_str())
        {
            return Err(protocol("merged fact keys are duplicated or out of order"));
        }
        if !current.is_empty()
            && encoded_shard_len(
                sequence,
                &current[0].record.fact_key,
                &item.fact.record.fact_key,
                current_records_bytes + item.fact.bytes.len(),
                current.len() + 1,
            )? > shard_limit
        {
            shards.push(publish_shard(store, sequence, &current, shard_limit)?);
            sequence = sequence.checked_add(1).ok_or_else(|| {
                ClewError::new(ErrorCode::ResourceLimit, "shard sequence overflow")
            })?;
            current.clear();
            current_records_bytes = 0;
        }
        if current.is_empty()
            && encoded_shard_len(
                sequence,
                &item.fact.record.fact_key,
                &item.fact.record.fact_key,
                item.fact.bytes.len(),
                1,
            )? > shard_limit
        {
            return Err(ClewError::new(
                ErrorCode::ResourceLimit,
                "one fact cannot fit in a canonical shard",
            ));
        }
        last_key = Some(item.fact.record.fact_key.clone());
        current_records_bytes += item.fact.bytes.len();
        current.push(item.fact);
        fact_count += 1;
        if let Some(next) = readers[item.run].next()? {
            heap.push(Reverse(HeapFact {
                run: item.run,
                fact: next,
            }));
        }
    }
    if !current.is_empty() {
        shards.push(publish_shard(store, sequence, &current, shard_limit)?);
    }
    if fact_count != expected_count || fact_count == 0 {
        return Err(protocol("merged generation fact count is incomplete"));
    }
    let mut manifest = GenerationManifest {
        schema: GENERATION_SCHEMA.into(),
        generation_id: String::new(),
        derived_input_manifest,
        generation_kind: if parent_generation.is_some() {
            GenerationKind::Delta
        } else {
            GenerationKind::Full
        },
        parent_generation,
        attempts,
        shards,
        fact_count,
    };
    manifest.generation_id = canonical::hash(&manifest).map_err(internal)?;
    let bytes = canonical::bytes(&manifest).map_err(internal)?;
    let object = store.put(GENERATION_SCHEMA, &bytes)?;
    Ok((manifest, object))
}

fn verify_attempts(store: &CasStore, attempts: &[AttemptAuthority]) -> Result<(), ClewError> {
    let mut scopes = BTreeSet::new();
    for attempt in attempts {
        if attempt.compilation_id.is_empty()
            || attempt.compilation_id.len() > 128
            || !canonical_digest(&attempt.completion.scope_digest)
            || !scopes.insert((
                attempt.compilation_id.as_str(),
                attempt.capability.as_str(),
                attempt.completion.scope_digest.as_str(),
            ))
        {
            return Err(invalid("generation attempt identity is invalid"));
        }
        let receipt = &attempt.completion.completeness_receipt;
        let limit = usize::try_from(receipt.size)
            .map_err(|_| ClewError::new(ErrorCode::ResourceLimit, "receipt exceeds host size"))?;
        store.read(receipt, limit)?;
    }
    Ok(())
}

fn canonical_digest(value: &str) -> bool {
    value.len() == 71
        && value.starts_with("sha256:")
        && value[7..]
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

fn publish_shard(
    store: &CasStore,
    sequence: u32,
    facts: &[EncodedFact],
    limit: usize,
) -> Result<CasObject, ClewError> {
    let shard = CanonicalFactShard {
        schema: FACT_SHARD_SCHEMA.into(),
        sequence,
        first_key: facts
            .first()
            .expect("non-empty shard")
            .record
            .fact_key
            .clone(),
        last_key: facts
            .last()
            .expect("non-empty shard")
            .record
            .fact_key
            .clone(),
        facts: facts.iter().map(|fact| fact.record.clone()).collect(),
    };
    let bytes = canonical::bytes(&shard).map_err(internal)?;
    if bytes.len() > limit {
        return Err(internal(
            "canonical shard size calculation disagrees with encoding",
        ));
    }
    store.put(FACT_SHARD_SCHEMA, &bytes)
}

fn encoded_shard_len(
    sequence: u32,
    first_key: &str,
    last_key: &str,
    records_bytes: usize,
    count: usize,
) -> Result<usize, ClewError> {
    let first = serde_json::to_vec(first_key).map_err(internal)?.len();
    let last = serde_json::to_vec(last_key).map_err(internal)?.len();
    Ok(b"{\"facts\":[".len()
        + records_bytes
        + count.saturating_sub(1)
        + b"],\"firstKey\":".len()
        + first
        + b",\"lastKey\":".len()
        + last
        + b",\"schema\":\"codeclew-canonical-fact-shard/2.0\",\"sequence\":".len()
        + sequence.to_string().len()
        + 1)
}

#[derive(Debug)]
struct EncodedFact {
    record: FactRecord,
    bytes: Vec<u8>,
}

struct RunReader {
    reader: BufReader<File>,
    remaining: u64,
}

impl RunReader {
    fn open(run: &FactRun) -> Result<Self, ClewError> {
        Ok(Self {
            reader: BufReader::new(File::open(&run.path).map_err(io_error)?),
            remaining: run.count,
        })
    }

    fn next(&mut self) -> Result<Option<EncodedFact>, ClewError> {
        if self.remaining == 0 {
            let mut trailing = [0u8; 1];
            if self.reader.read(&mut trailing).map_err(io_error)? != 0 {
                return Err(protocol("fact run contains trailing bytes"));
            }
            return Ok(None);
        }
        let mut length = [0u8; 4];
        self.reader.read_exact(&mut length).map_err(io_error)?;
        let length = u32::from_be_bytes(length) as usize;
        if length == 0 || length > MAX_FACT_BYTES {
            return Err(protocol("fact run record length is invalid"));
        }
        let mut bytes = vec![0; length];
        self.reader.read_exact(&mut bytes).map_err(io_error)?;
        let record: FactRecord =
            serde_json::from_slice(&bytes).map_err(|_| protocol("fact run record is invalid"))?;
        record.validate()?;
        if canonical::bytes(&record).map_err(internal)? != bytes {
            return Err(protocol("fact run record is not canonical"));
        }
        self.remaining -= 1;
        Ok(Some(EncodedFact { record, bytes }))
    }
}

struct HeapFact {
    run: usize,
    fact: EncodedFact,
}

impl PartialEq for HeapFact {
    fn eq(&self, other: &Self) -> bool {
        (self.fact.record.fact_key.as_str(), self.run)
            == (other.fact.record.fact_key.as_str(), other.run)
    }
}

impl Eq for HeapFact {}

impl PartialOrd for HeapFact {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for HeapFact {
    fn cmp(&self, other: &Self) -> Ordering {
        (self.fact.record.fact_key.as_str(), self.run)
            .cmp(&(other.fact.record.fact_key.as_str(), other.run))
    }
}

fn invalid(message: &str) -> ClewError {
    ClewError::new(ErrorCode::InvalidInput, message)
}

fn protocol(message: &str) -> ClewError {
    ClewError::new(ErrorCode::WorkerProtocolMismatch, message)
}

fn corrupt(message: &str) -> ClewError {
    ClewError::new(ErrorCode::StateCorrupt, message)
}

fn internal(error: impl std::fmt::Display) -> ClewError {
    ClewError::new(ErrorCode::Internal, error.to_string())
}

fn io_error(error: std::io::Error) -> ClewError {
    ClewError::new(ErrorCode::Internal, error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cas::CasStore;
    use rayon::prelude::*;

    fn fixture() -> (
        tempfile::TempDir,
        StateAuthority,
        CasStore,
        CasObject,
        AttemptAuthority,
    ) {
        let root = tempfile::tempdir().unwrap();
        let state = StateAuthority::open(root.path().join("v2")).unwrap();
        let store = CasStore::open(&state).unwrap();
        let derived = store.put(DERIVED_MANIFEST_SCHEMA, b"derived").unwrap();
        let receipt = store.put("test/receipt/1", b"complete").unwrap();
        let attempt = AttemptAuthority {
            compilation_id: "main".into(),
            capability: CapabilityUri::parse("analysis:facts").unwrap(),
            completion: AnalysisAttemptComplete {
                scope_digest: format!("sha256:{}", "a".repeat(64)),
                completeness_receipt: receipt,
                fact_count: 6,
            },
        };
        (root, state, store, derived, attempt)
    }

    fn fact(store: &CasStore, key: &str) -> FactRecord {
        FactRecord {
            fact_key: key.into(),
            domain_uri: CapabilityUri::parse("analysis:facts").unwrap(),
            payload: store.put("test/fact-payload/1", key.as_bytes()).unwrap(),
        }
    }

    fn run(state: &StateAuthority, facts: &[FactRecord]) -> FactRun {
        let mut writer = FactRunWriter::create(state).unwrap();
        for fact in facts {
            writer.push(fact).unwrap();
        }
        writer.finish().unwrap()
    }

    fn parallel_runs(state: &StateAuthority, facts: &[FactRecord], jobs: usize) -> Vec<FactRun> {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(jobs)
            .build()
            .unwrap();
        let chunk_size = facts.len().div_ceil(jobs);
        pool.install(|| {
            facts
                .par_chunks(chunk_size)
                .map(|chunk| run(state, chunk))
                .collect()
        })
    }

    #[test]
    fn run_arrival_order_cannot_change_shards_or_generation_digest() {
        let (_root, state, store, derived, attempt) = fixture();
        let facts = ["a", "b", "c", "d", "e", "f"].map(|key| fact(&store, key));
        let first_runs = vec![run(&state, &facts)];
        let (first, first_object) = finalize_with_limit(
            &store,
            derived.clone(),
            vec![attempt.clone()],
            first_runs,
            None,
            600,
        )
        .unwrap();
        let second_runs = vec![
            run(
                &state,
                &[facts[1].clone(), facts[3].clone(), facts[5].clone()],
            ),
            run(
                &state,
                &[facts[0].clone(), facts[2].clone(), facts[4].clone()],
            ),
        ];
        let (second, second_object) =
            finalize_with_limit(&store, derived, vec![attempt], second_runs, None, 600).unwrap();
        assert_eq!(first, second);
        assert_eq!(first_object, second_object);
        assert!(first.shards.len() > 1);
        first.verify(&store).unwrap();
        for shard in &first.shards {
            assert!(shard.size <= 600);
        }
    }

    #[test]
    fn jobs_one_and_jobs_n_publish_byte_identical_generation() {
        let (_root, state, store, derived, attempt) = fixture();
        let facts = ["a", "b", "c", "d", "e", "f"].map(|key| fact(&store, key));
        let (single, single_object) = finalize_with_limit(
            &store,
            derived.clone(),
            vec![attempt.clone()],
            parallel_runs(&state, &facts, 1),
            None,
            600,
        )
        .unwrap();
        let (parallel, parallel_object) = finalize_with_limit(
            &store,
            derived,
            vec![attempt],
            parallel_runs(&state, &facts, 3),
            None,
            600,
        )
        .unwrap();
        assert_eq!(single, parallel);
        assert_eq!(single_object, parallel_object);
    }

    #[test]
    fn duplicate_fact_key_fails_closed() {
        let (_root, state, store, derived, attempt) = fixture();
        let same = fact(&store, "same");
        let runs = vec![
            run(&state, std::slice::from_ref(&same)),
            run(&state, &[same]),
        ];
        assert_eq!(
            finalize_with_limit(&store, derived, vec![attempt], runs, None, 600)
                .unwrap_err()
                .code,
            ErrorCode::WorkerProtocolMismatch
        );
    }

    #[test]
    fn delta_generation_is_immutably_bound_to_available_parent() {
        let (_root, state, store, derived, attempt) = fixture();
        let parent_facts = ["a", "b", "c", "d", "e", "f"].map(|key| fact(&store, key));
        let (_parent, parent_object) = finalize_generation(
            &store,
            derived.clone(),
            vec![attempt.clone()],
            vec![run(&state, &parent_facts)],
        )
        .unwrap();
        let (delta, _) = finalize_generation_with_parent(
            &store,
            derived,
            vec![attempt],
            vec![run(&state, &parent_facts)],
            Some(parent_object.clone()),
        )
        .unwrap();
        assert_eq!(delta.generation_kind, GenerationKind::Delta);
        assert_eq!(delta.parent_generation, Some(parent_object));
        delta.verify(&store).unwrap();
    }
}
