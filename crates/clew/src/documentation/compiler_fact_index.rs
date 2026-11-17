//! Disposable producer index. CAS remains authority; payload residency is bounded.
use super::super::{progress, store};
use crate::{
    canonical,
    cas::{CasObject, CasStore},
    error::{ClewError, ErrorCode},
    generation_service::ReadyGenerationSet,
    generation_v2::GenerationManifest,
};
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{borrow::Cow, collections::VecDeque};

pub(super) const MAX_RECORD_BYTES: u64 = 8 * 1024 * 1024;
const BATCH_BYTES: usize = 8 * 1024 * 1024;
const BATCH_FACTS: usize = 256;
type Fact = (Value, String);

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub(super) enum Stage {
    AdmitFactReferences,
    HydrateFactPayloads,
    ProjectFacts,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub(super) enum Dimension {
    RecordPayloadBytes,
    CounterOverflow,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct Budget {
    schema: String,
    stage: Stage,
    observed_facts: u64,
    observed_payload_bytes: u64,
    planned_facts: Option<u64>,
    planned_payload_bytes: Option<u64>,
    compilation_ordinal: Option<usize>,
    compilations_total: usize,
    current_compilation_facts: Option<u64>,
    current_compilation_payload_bytes: Option<u64>,
    current_record_ordinal: Option<u64>,
    current_record_payload_bytes: Option<u64>,
    current_record_size_verified: bool,
    storage_code: Option<i32>,
    max_record_payload_bytes: u64,
    max_batch_facts: usize,
    max_batch_payload_bytes: usize,
    exceeded: Vec<Dimension>,
}

impl Budget {
    fn new(compilations: usize) -> Self {
        Self {
            schema: "codeclew-documentation-fact-budget/1.0".into(),
            stage: Stage::AdmitFactReferences,
            observed_facts: 0,
            observed_payload_bytes: 0,
            planned_facts: None,
            planned_payload_bytes: None,
            compilation_ordinal: None,
            compilations_total: compilations,
            current_compilation_facts: None,
            current_compilation_payload_bytes: None,
            current_record_ordinal: None,
            current_record_payload_bytes: None,
            current_record_size_verified: false,
            storage_code: None,
            max_record_payload_bytes: MAX_RECORD_BYTES,
            max_batch_facts: BATCH_FACTS,
            max_batch_payload_bytes: BATCH_BYTES,
            exceeded: vec![],
        }
    }

    fn counts(&self) -> progress::Counts {
        progress::Counts {
            compilation_ordinal: self.compilation_ordinal,
            compilations_total: Some(self.compilations_total),
            facts: Some(self.observed_facts),
            payload_bytes: Some(self.observed_payload_bytes),
            planned_facts: self.planned_facts,
            planned_payload_bytes: self.planned_payload_bytes,
            ..Default::default()
        }
    }

    fn attach(&self, mut error: ClewError) -> ClewError {
        // A closed numeric schema only. Never copy arbitrary error details.
        let mut diagnostic = self.clone();
        if let Some(code) = error
            .details
            .as_ref()
            .filter(|v| v["schema"] == "codeclew-documentation-fact-index-storage/1.0")
            .and_then(|v| v["sqliteCode"].as_i64())
            .and_then(|n| i32::try_from(n).ok())
        {
            diagnostic.storage_code = Some(code);
        }
        error
            .evidence
            .push(serde_json::to_string(&diagnostic).expect("numeric diagnostic"));
        error
    }

    fn add(&mut self, size: u64) -> Result<(), ClewError> {
        let next = self
            .observed_facts
            .checked_add(1)
            .zip(self.observed_payload_bytes.checked_add(size));
        let Some((count, bytes)) = next else {
            self.exceeded.push(Dimension::CounterOverflow);
            return Err(ClewError::new(
                ErrorCode::ResourceLimit,
                "documentation fact counters exceed numeric representation",
            ));
        };
        if count > i64::MAX as u64 {
            self.exceeded.push(Dimension::CounterOverflow);
            return Err(ClewError::new(
                ErrorCode::ResourceLimit,
                "documentation fact index ordinal exceeds numeric representation",
            ));
        }
        self.observed_facts = count;
        self.observed_payload_bytes = bytes;
        Ok(())
    }
}

pub(in crate::documentation) fn summary(value: &Value) -> Option<Value> {
    let budget: Budget = serde_json::from_value(value.clone()).ok()?;
    (budget.schema == "codeclew-documentation-fact-budget/1.0"
        && budget.max_record_payload_bytes == MAX_RECORD_BYTES
        && budget.max_batch_facts == BATCH_FACTS
        && budget.max_batch_payload_bytes == BATCH_BYTES)
        .then(|| serde_json::to_value(budget).expect("numeric diagnostic"))
}

pub(in crate::documentation) fn diagnostic(error: &ClewError) -> Option<Value> {
    error.evidence.iter().find_map(|raw| {
        (raw.len() <= 4096)
            .then(|| serde_json::from_str::<Value>(raw).ok())
            .flatten()
            .and_then(|v| summary(&v))
    })
}

pub(super) struct FactIndex {
    connection: Connection,
    _directory: tempfile::TempDir,
    count: u64,
    batch_facts: usize,
    budget: Budget,
}

impl FactIndex {
    fn empty() -> Result<Self, ClewError> {
        let directory = tempfile::Builder::new()
            .prefix("clew-docs-facts-")
            .tempdir()
            .map_err(io_error)?;
        let connection =
            Connection::open(directory.path().join("facts.sqlite")).map_err(io_error)?;
        connection.execute_batch(
            "PRAGMA journal_mode=OFF; PRAGMA synchronous=OFF; PRAGMA cache_size=-2048; PRAGMA mmap_size=0; PRAGMA temp_store=FILE;
             CREATE TABLE facts (ordinal INTEGER PRIMARY KEY, compilation INTEGER NOT NULL, scope TEXT NOT NULL, reference TEXT NOT NULL, reference_hash TEXT NOT NULL, payload BLOB, row_hash TEXT);",
        ).map_err(io_error)?;
        Ok(Self {
            connection,
            _directory: directory,
            count: 0,
            batch_facts: BATCH_FACTS,
            budget: Budget::new(0),
        })
    }

    #[cfg(test)]
    pub(super) fn from_memory_for_test(facts: &[Fact]) -> Result<Self, ClewError> {
        let mut index = Self::empty()?;
        // Split every pair of facts so global closure cannot accidentally rely
        // on declarations, accesses or annotations sharing a payload batch.
        index.batch_facts = 1;
        for (ordinal, (value, binding)) in facts.iter().enumerate() {
            let payload = serde_json::to_vec(value).map_err(io_error)?;
            let mut reference = CasObject::for_bytes("owned-fact/1.0", &payload)?;
            reference.digest = binding.clone();
            index
                .connection
                .execute(
                    "INSERT INTO facts VALUES(?1,1,'owned',?2,?3,?4,?5)",
                    params![
                        (ordinal + 1) as i64,
                        serde_json::to_string(&reference).map_err(io_error)?,
                        reference_digest((ordinal + 1) as u64, 1, "owned", &reference)?,
                        payload,
                        row_digest(
                            &reference_digest((ordinal + 1) as u64, 1, "owned", &reference)?,
                            &payload
                        )?
                    ],
                )
                .map_err(io_error)?;
        }
        index.count = facts.len() as u64;
        Ok(index)
    }

    pub(super) fn from_generations(
        store: &CasStore,
        ready: &ReadyGenerationSet,
        domain: &str,
    ) -> Result<Self, ClewError> {
        let references: Vec<_> = ready
            .compilations
            .iter()
            .map(|c| (c.compilation.as_str(), &c.generation))
            .collect();
        Self::from_references(store, &references, domain)
    }

    fn from_references(
        store: &CasStore,
        compilations: &[(&str, &CasObject)],
        domain: &str,
    ) -> Result<Self, ClewError> {
        let mut budget = Budget::new(compilations.len());
        let result = (|| {
            let mut index = Self::empty()?;
            let mut compilation_totals = Vec::with_capacity(compilations.len());
            progress::run_with_phase("ADMIT_FACT_REFERENCES", |phase| {
                index.connection.execute_batch("BEGIN").map_err(io_error)?;
                for (ordinal, (scope, generation)) in compilations.iter().enumerate() {
                    budget.compilation_ordinal = Some(ordinal + 1);
                    budget.current_compilation_facts = Some(0);
                    budget.current_compilation_payload_bytes = Some(0);
                    budget.current_record_ordinal = None;
                    budget.current_record_payload_bytes = None;
                    phase.set_counts(budget.counts());
                    let lease = store.read(generation, store::MAX_RECORD as usize)?;
                    let generation: GenerationManifest =
                        serde_json::from_slice(lease.bytes()).map_err(|_| corrupt())?;
                    generation.visit_facts(store, |fact| {
                        if fact.domain_uri.as_str() != domain { return Ok(()); }
                        budget.add(fact.payload.size)?;
                        budget.current_compilation_facts = budget.current_compilation_facts.map(|n| n+1);
                        budget.current_compilation_payload_bytes = budget.current_compilation_payload_bytes.map(|n| n+fact.payload.size);
                        budget.current_record_ordinal = Some(budget.observed_facts);
                        budget.current_record_payload_bytes = Some(fact.payload.size);
                        phase.set_counts(budget.counts());
                        index.connection.execute("INSERT INTO facts(ordinal,compilation,scope,reference,reference_hash) VALUES (?1,?2,?3,?4,?5)",
                            params![budget.observed_facts as i64, (ordinal + 1) as i64, scope, serde_json::to_string(&fact.payload).map_err(io_error)?, reference_digest(budget.observed_facts, ordinal+1, scope, &fact.payload)?]).map_err(io_error)?;
                        Ok(())
                    })?;
                    compilation_totals.push((
                        budget.current_compilation_facts,
                        budget.current_compilation_payload_bytes,
                    ));
                }
                index.connection.execute_batch("COMMIT").map_err(io_error)?;
                index.count = budget.observed_facts;
                budget.planned_facts = Some(budget.observed_facts);
                budget.planned_payload_bytes = Some(budget.observed_payload_bytes);
                phase.set_counts(budget.counts());
                // Exact matching-domain metadata totals now exist. Check every
                // record before hydrating any JSON, independently of service totals.
                let mut statement = index
                    .connection
                    .prepare("SELECT ordinal,compilation,reference FROM facts ORDER BY ordinal")
                    .map_err(io_error)?;
                let mut rows = statement.query([]).map_err(io_error)?;
                while let Some(row) = rows.next().map_err(io_error)? {
                    let reference: CasObject =
                        serde_json::from_str(&row.get::<_, String>(2).map_err(io_error)?)
                            .map_err(io_error)?;
                    if reference.size > MAX_RECORD_BYTES {
                        let compilation = usize::try_from(row.get::<_, i64>(1).map_err(io_error)?)
                            .map_err(io_error)?;
                        budget.compilation_ordinal = Some(compilation);
                        budget.current_record_ordinal = Some(
                            u64::try_from(row.get::<_, i64>(0).map_err(io_error)?)
                                .map_err(io_error)?,
                        );
                        budget.current_record_payload_bytes = Some(reference.size);
                        (
                            budget.current_compilation_facts,
                            budget.current_compilation_payload_bytes,
                        ) = compilation_totals[compilation - 1];
                        budget.exceeded.push(Dimension::RecordPayloadBytes);
                        return Err(ClewError::new(
                            ErrorCode::SliceBudgetExceeded,
                            "documentation fact record exceeds the bounded producer payload unit",
                        ));
                    }
                }
                Ok(())
            })?;
            budget.stage = Stage::HydrateFactPayloads;
            budget.observed_facts = 0;
            budget.observed_payload_bytes = 0;
            progress::run_with_phase("HYDRATE_FACT_PAYLOADS", |phase| {
                let mut offset = 0u64;
                while offset < index.count {
                    let references = {
                        let mut stmt = index.connection.prepare("SELECT ordinal,compilation,scope,reference,reference_hash FROM facts WHERE ordinal>?1 ORDER BY ordinal LIMIT ?2").map_err(io_error)?;
                        stmt.query_map(params![offset as i64, BATCH_FACTS as i64], |row| {
                            Ok((
                                row.get::<_, i64>(0)?,
                                row.get::<_, i64>(1)?,
                                row.get::<_, String>(2)?,
                                row.get::<_, String>(3)?,
                                row.get::<_, String>(4)?,
                            ))
                        })
                        .map_err(io_error)?
                        .collect::<Result<Vec<_>, _>>()
                        .map_err(io_error)?
                    };
                    if references.is_empty() {
                        return Err(corrupt());
                    }
                    index.connection.execute_batch("BEGIN").map_err(io_error)?;
                    for (ordinal, compilation, scope, raw_reference, metadata_hash) in references {
                        let ordinal = u64::try_from(ordinal).map_err(io_error)?;
                        let compilation = usize::try_from(compilation).map_err(io_error)?;
                        let reference: CasObject =
                            serde_json::from_str(&raw_reference).map_err(|_| corrupt())?;
                        if compilation == 0
                            || compilation > compilations.len()
                            || scope != compilations[compilation - 1].0
                            || reference_digest(ordinal, compilation, &scope, &reference)?
                                != metadata_hash
                        {
                            return Err(corrupt());
                        }
                        if budget.compilation_ordinal != Some(compilation) || offset == 0 {
                            budget.current_compilation_facts = Some(0);
                            budget.current_compilation_payload_bytes = Some(0);
                        }
                        budget.compilation_ordinal = Some(compilation);
                        budget.current_record_ordinal = Some(ordinal);
                        budget.current_record_payload_bytes = Some(reference.size);
                        budget.current_record_size_verified = false;
                        let lease = store.read(&reference, MAX_RECORD_BYTES as usize)?;
                        budget.current_record_size_verified = true;
                        let mut value: Value =
                            serde_json::from_slice(lease.bytes()).map_err(|_| corrupt())?;
                        if !value.is_object() {
                            return Err(corrupt());
                        }
                        value["scope"] = json!({"compilation":scope});
                        let payload = serde_json::to_vec(&value).map_err(io_error)?;
                        index
                            .connection
                            .execute(
                                "UPDATE facts SET payload=?1,row_hash=?2 WHERE ordinal=?3",
                                params![
                                    payload,
                                    row_digest(&metadata_hash, &payload)?,
                                    ordinal as i64
                                ],
                            )
                            .map_err(io_error)?;
                        budget.add(reference.size)?;
                        budget.current_compilation_facts =
                            budget.current_compilation_facts.map(|n| n + 1);
                        budget.current_compilation_payload_bytes = budget
                            .current_compilation_payload_bytes
                            .map(|n| n + reference.size);
                        phase.set_counts(budget.counts());
                        offset = ordinal;
                    }
                    index.connection.execute_batch("COMMIT").map_err(io_error)?;
                }
                if Some(budget.observed_facts) != budget.planned_facts
                    || Some(budget.observed_payload_bytes) != budget.planned_payload_bytes
                {
                    return Err(corrupt());
                }
                Ok(())
            })?;
            index.budget = budget.clone();
            Ok(index)
        })();
        result.map_err(|e| budget.attach(e))
    }
}

fn reference_digest(
    ordinal: u64,
    compilation: usize,
    scope: &str,
    reference: &CasObject,
) -> Result<String, ClewError> {
    canonical::hash(&(ordinal, compilation, scope, reference)).map_err(io_error)
}

fn row_digest(reference_hash: &str, payload: &[u8]) -> Result<String, ClewError> {
    canonical::hash(&(reference_hash, canonical::hash_bytes(payload))).map_err(io_error)
}

fn io_error(error: impl std::fmt::Display + 'static) -> ClewError {
    // Preserve concrete storage/integrity causes before the generic adapter
    // would erase SQLite's typed result. Arbitrary messages stay out of public
    // diagnostics; the numeric SQLite code is safe to persist with the stage.
    if let Some(rusqlite::Error::SqliteFailure(code, _)) =
        (&error as &dyn std::any::Any).downcast_ref::<rusqlite::Error>()
    {
        let primary = code.extended_code & 255;
        let kind = match primary {
            rusqlite::ffi::SQLITE_FULL
            | rusqlite::ffi::SQLITE_NOMEM
            | rusqlite::ffi::SQLITE_TOOBIG => ErrorCode::ResourceLimit,
            rusqlite::ffi::SQLITE_CORRUPT | rusqlite::ffi::SQLITE_NOTADB => ErrorCode::StateCorrupt,
            _ => ErrorCode::Internal,
        };
        let mut result = ClewError::new(kind, "documentation temporary fact index storage failed");
        result.details = Some(Box::new(
            json!({"schema":"codeclew-documentation-fact-index-storage/1.0", "sqliteCode":code.extended_code}),
        ));
        return result;
    }
    super::super::io_error(error)
}

#[cfg(test)]
pub(in crate::documentation) fn refusal_fixture() -> ClewError {
    let mut budget = Budget::new(3);
    budget.stage = Stage::HydrateFactPayloads;
    budget.observed_facts = 41;
    budget.observed_payload_bytes = 900;
    budget.planned_facts = Some(100);
    budget.planned_payload_bytes = Some(2000);
    budget.compilation_ordinal = Some(2);
    budget.current_record_ordinal = Some(42);
    budget.current_record_payload_bytes = Some(MAX_RECORD_BYTES + 1);
    budget.exceeded.push(Dimension::RecordPayloadBytes);
    budget.attach(ClewError::new(
        ErrorCode::SliceBudgetExceeded,
        "owned fact payload unit refused",
    ))
}

#[cfg(test)]
pub(in crate::documentation) fn integrity_refusal_fixture() -> ClewError {
    let index = FactIndex::from_memory_for_test(&[
        (json!({"n":1}), "first".into()),
        (json!({"n":2}), "second".into()),
    ])
    .unwrap();
    index
        .connection
        .execute("DELETE FROM facts WHERE ordinal=2", [])
        .unwrap();
    let facts = Facts::Indexed(&index);
    let mut rows = facts.iter();
    assert!(rows.next().unwrap().is_ok());
    rows.next().unwrap().err().unwrap()
}

#[cfg(test)]
pub(in crate::documentation) fn storage_refusal_fixture() -> ClewError {
    let index = FactIndex::empty().unwrap();
    index
        .connection
        .execute_batch("PRAGMA max_page_count=2")
        .unwrap();
    let failure = index
        .connection
        .execute(
            "INSERT INTO facts VALUES(1,1,'owned','{}','ref',zeroblob(1048576),'row')",
            [],
        )
        .unwrap_err();
    let mut budget = Budget::new(2);
    budget.stage = Stage::HydrateFactPayloads;
    budget.observed_facts = 41;
    budget.observed_payload_bytes = 900;
    budget.planned_facts = Some(100);
    budget.planned_payload_bytes = Some(2000);
    budget.attach(io_error(failure))
}

fn corrupt() -> ClewError {
    ClewError::new(
        ErrorCode::StateCorrupt,
        "documentation temporary fact index is incomplete or corrupt",
    )
}

pub(super) enum Facts<'a> {
    Memory(&'a [Fact]),
    Indexed(&'a FactIndex),
}

impl Facts<'_> {
    pub(super) fn len(&self) -> u64 {
        match self {
            Self::Memory(rows) => rows.len() as u64,
            Self::Indexed(index) => index.count,
        }
    }

    pub(super) fn iter(&self) -> FactIter<'_> {
        match self {
            Self::Memory(rows) => FactIter::Memory(rows.iter()),
            Self::Indexed(index) => FactIter::Indexed {
                index,
                offset: 0,
                buffered: VecDeque::new(),
                failed: false,
                read_payload_bytes: 0,
            },
        }
    }
}

type StoredRow = (i64, Vec<u8>, String, String, usize, String, String);
pub(super) enum FactIter<'a> {
    Memory(std::slice::Iter<'a, Fact>),
    Indexed {
        index: &'a FactIndex,
        offset: u64,
        buffered: VecDeque<StoredRow>,
        failed: bool,
        read_payload_bytes: u64,
    },
}

impl<'a> Iterator for FactIter<'a> {
    type Item = Result<Cow<'a, Fact>, ClewError>;
    fn next(&mut self) -> Option<Self::Item> {
        match self {
            Self::Memory(rows) => rows.next().map(|v| Ok(Cow::Borrowed(v))),
            Self::Indexed {
                index,
                offset,
                buffered,
                failed,
                read_payload_bytes,
            } => {
                if *failed {
                    return None;
                }
                let result = (|| {
                    if buffered.is_empty() {
                        let mut stmt = index.connection.prepare("SELECT ordinal,payload,row_hash,reference,compilation,scope,reference_hash FROM facts WHERE ordinal>?1 ORDER BY ordinal LIMIT ?2").map_err(io_error)?;
                        let mut rows = stmt
                            .query(params![*offset as i64, index.batch_facts as i64])
                            .map_err(io_error)?;
                        let mut bytes = 0usize;
                        while let Some(row) = rows.next().map_err(io_error)? {
                            let payload: Vec<u8> = row.get(1).map_err(io_error)?;
                            if !buffered.is_empty()
                                && bytes.saturating_add(payload.len()) > BATCH_BYTES
                            {
                                break;
                            }
                            if payload.len() > (MAX_RECORD_BYTES as usize).saturating_add(65536) {
                                return Err(corrupt());
                            }
                            bytes += payload.len();
                            buffered.push_back((
                                row.get(0).map_err(io_error)?,
                                payload,
                                row.get(2).map_err(io_error)?,
                                row.get(3).map_err(io_error)?,
                                usize::try_from(row.get::<_, i64>(4).map_err(io_error)?)
                                    .map_err(io_error)?,
                                row.get(5).map_err(io_error)?,
                                row.get(6).map_err(io_error)?,
                            ));
                        }
                    }
                    let Some((
                        ordinal,
                        payload,
                        row_hash,
                        reference,
                        compilation,
                        scope,
                        reference_hash,
                    )) = buffered.pop_front()
                    else {
                        return if *offset == index.count {
                            Ok(None)
                        } else {
                            Err(corrupt())
                        };
                    };
                    let ordinal = u64::try_from(ordinal).map_err(io_error)?;
                    let reference: CasObject =
                        serde_json::from_str(&reference).map_err(|_| corrupt())?;
                    if ordinal != *offset + 1
                        || reference_digest(ordinal, compilation, &scope, &reference)?
                            != reference_hash
                        || row_digest(&reference_hash, &payload)? != row_hash
                    {
                        return Err(corrupt());
                    }
                    let value = serde_json::from_slice(&payload).map_err(|_| corrupt())?;
                    *offset = ordinal;
                    *read_payload_bytes = read_payload_bytes
                        .checked_add(reference.size)
                        .ok_or_else(corrupt)?;
                    Ok(Some(Cow::Owned((value, reference.digest))))
                })();
                match result {
                    Ok(Some(value)) => Some(Ok(value)),
                    Ok(None) => None,
                    Err(error) => {
                        *failed = true;
                        let mut budget = index.budget.clone();
                        budget.stage = Stage::ProjectFacts;
                        budget.observed_facts = *offset;
                        budget.observed_payload_bytes = *read_payload_bytes;
                        budget.planned_facts = Some(index.count);
                        budget.compilation_ordinal = None;
                        budget.current_compilation_facts = None;
                        budget.current_compilation_payload_bytes = None;
                        budget.current_record_ordinal = None;
                        budget.current_record_payload_bytes = None;
                        budget.current_record_size_verified = false;
                        Some(Err(budget.attach(error)))
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        adapter_v2::{AnalysisAttemptComplete, CapabilityUri, FactRecord},
        derived_manifest::DERIVED_MANIFEST_SCHEMA,
        generation_v2::{AttemptAuthority, FactRunWriter, finalize_generation},
        state::StateAuthority,
    };

    fn generation(state: &StateAuthority, store: &CasStore, rows: &[(Value, &str)]) -> CasObject {
        let mut run = FactRunWriter::create(state).unwrap();
        for (i, (value, domain)) in rows.iter().enumerate() {
            run.push(&FactRecord {
                fact_key: format!("fact:{i:08}"),
                domain_uri: CapabilityUri::parse(*domain).unwrap(),
                payload: store
                    .put("owned-fact/1.0", &serde_json::to_vec(value).unwrap())
                    .unwrap(),
            })
            .unwrap();
        }
        let derived = store
            .put(DERIVED_MANIFEST_SCHEMA, b"owned derived")
            .unwrap();
        let receipt = store.put("owned-receipt/1.0", b"owned receipt").unwrap();
        finalize_generation(
            store,
            derived,
            vec![AttemptAuthority {
                compilation_id: "owned/main".into(),
                capability: CapabilityUri::parse("analysis:java-compiler-facts").unwrap(),
                completion: AnalysisAttemptComplete {
                    scope_digest: format!("sha256:{}", "a".repeat(64)),
                    completeness_receipt: receipt,
                    fact_count: rows.len() as u64,
                },
            }],
            vec![run.finish().unwrap()],
        )
        .unwrap()
        .1
    }

    #[test]
    fn exact_matching_domain_and_repeated_scopes_survive_bounded_replay() {
        let root = tempfile::tempdir().unwrap();
        let state = StateAuthority::open(root.path().join("state")).unwrap();
        let store = CasStore::open(&state).unwrap();
        let rows: Vec<_> = (0..600).map(|n| (json!({"kind":"DECLARATION","file":"Shared.java","symbolIdentity":format!("symbol:{n}"),"payload":"x".repeat(64)}), if n % 3 == 0 {"analysis:other"} else {"analysis:java-compiler-facts"})).collect();
        let generation = generation(&state, &store, &rows);
        let index = FactIndex::from_references(
            &store,
            &[(":a/main", &generation), (":b/main", &generation)],
            "analysis:java-compiler-facts",
        )
        .unwrap();
        assert_eq!(index.count, 800);
        let first: Vec<_> = Facts::Indexed(&index)
            .iter()
            .map(|r| r.unwrap().into_owned())
            .collect();
        let second: Vec<_> = Facts::Indexed(&index)
            .iter()
            .map(|r| r.unwrap().into_owned())
            .collect();
        assert_eq!(first, second);
        for n in 0..400 {
            assert_eq!(first[n].0["scope"]["compilation"], ":a/main");
            assert_eq!(first[n + 400].0["scope"]["compilation"], ":b/main");
            assert_eq!(first[n].1, first[n + 400].1);
            let mut a = first[n].0.clone();
            let mut b = first[n + 400].0.clone();
            a.as_object_mut().unwrap().remove("scope");
            b.as_object_mut().unwrap().remove("scope");
            assert_eq!(a, b);
        }
    }

    #[test]
    fn former_aggregate_boundaries_are_not_payload_working_units() {
        for count in [524_287u64, 524_288, 524_289] {
            for bytes in [268_435_455u64, 268_435_456, 268_435_457] {
                let mut b = Budget::new(2);
                b.observed_facts = count - 1;
                b.observed_payload_bytes = bytes - 1024;
                b.add(1024).unwrap();
                assert_eq!((b.observed_facts, b.observed_payload_bytes), (count, bytes));
                assert!(b.exceeded.is_empty());
            }
        }
    }

    #[test]
    fn early_record_refusal_reports_full_metadata_before_any_payload_hydration() {
        let root = tempfile::tempdir().unwrap();
        let state = StateAuthority::open(root.path().join("state")).unwrap();
        let store = CasStore::open(&state).unwrap();
        let generation = generation(
            &state,
            &store,
            &[(
                json!({"padding":"x".repeat(MAX_RECORD_BYTES as usize-13)}),
                "analysis:java-compiler-facts",
            )],
        );
        let error = FactIndex::from_references(
            &store,
            &[(":a/main", &generation)],
            "analysis:java-compiler-facts",
        )
        .err()
        .unwrap();
        assert_eq!(error.code, ErrorCode::SliceBudgetExceeded);
        let d = diagnostic(&error).unwrap();
        assert_eq!(d["stage"], "ADMIT_FACT_REFERENCES");
        assert_eq!(d["plannedFacts"], 1);
        assert_eq!(d["plannedPayloadBytes"], MAX_RECORD_BYTES + 1);
        assert_eq!(d["compilationOrdinal"], 1);
        assert_eq!(d["currentRecordOrdinal"], 1);
        assert_eq!(d["currentRecordPayloadBytes"], d["plannedPayloadBytes"]);
        assert_eq!(d["currentRecordSizeVerified"], false);
        assert_eq!(d["exceeded"], json!(["RECORD_PAYLOAD_BYTES"]));
        let mut forged = d;
        forged["path"] = json!("private marker");
        assert!(summary(&forged).is_none());
    }

    #[test]
    fn missing_and_corrupt_temporary_rows_never_end_as_complete() {
        let index = FactIndex::empty().unwrap();
        index
            .connection
            .execute(
                "INSERT INTO facts VALUES(1,1,'scope','{}','wrong-ref',X'7B7D','wrong')",
                [],
            )
            .unwrap();
        let index = FactIndex { count: 1, ..index };
        assert!(Facts::Indexed(&index).iter().next().unwrap().is_err());
        index.connection.execute("DELETE FROM facts", []).unwrap();
        assert!(Facts::Indexed(&index).iter().next().unwrap().is_err());
    }
    fn manifest(store: &CasStore, mut value: GenerationManifest) -> CasObject {
        value.generation_id.clear();
        value.generation_id = canonical::hash(&value).unwrap();
        store
            .put(
                crate::generation_v2::GENERATION_SCHEMA,
                &canonical::bytes(&value).unwrap(),
            )
            .unwrap()
    }

    #[test]
    fn genuine_generation_parts_and_references_reject_integrity_failures() {
        let root = tempfile::tempdir().unwrap();
        let state = StateAuthority::open(root.path().join("state")).unwrap();
        let store = CasStore::open(&state).unwrap();
        let original = generation(
            &state,
            &store,
            &[
                (json!({"n":1}), "analysis:java-compiler-facts"),
                (json!({"n":2}), "analysis:java-compiler-facts"),
            ],
        );
        let lease = store.read(&original, 1024 * 1024).unwrap();
        let mut base: GenerationManifest = serde_json::from_slice(lease.bytes()).unwrap();
        let lease = store.read(&base.shards[0], 1024 * 1024).unwrap();
        let full: Value = serde_json::from_slice(lease.bytes()).unwrap();
        base.shards=(0..2).map(|n| {
            let fact=full["facts"][n].clone();
            let shard=json!({"schema":crate::generation_v2::FACT_SHARD_SCHEMA,"sequence":n,"firstKey":fact["factKey"],"lastKey":fact["factKey"],"facts":[fact]});
            store.put(crate::generation_v2::FACT_SHARD_SCHEMA,&canonical::bytes(&shard).unwrap()).unwrap()
        }).collect();
        let good = manifest(&store, base.clone());
        assert_eq!(
            FactIndex::from_references(&store, &[("one", &good)], "analysis:java-compiler-facts")
                .unwrap()
                .count,
            2
        );
        let mut cases = vec![];
        let mut bad = base.clone();
        bad.shards.swap(0, 1);
        cases.push(("reordered", bad));
        let mut bad = base.clone();
        bad.shards[1] = bad.shards[0].clone();
        cases.push(("duplicate", bad));
        let mut bad = base.clone();
        bad.fact_count += 1;
        cases.push(("count", bad));
        let mut bad = base.clone();
        bad.shards[1] =
            CasObject::for_bytes(crate::generation_v2::FACT_SHARD_SCHEMA, b"not stored").unwrap();
        cases.push(("missing", bad));
        let mut bad = base.clone();
        bad.shards[1] = store
            .put(crate::generation_v2::FACT_SHARD_SCHEMA, b"not JSON")
            .unwrap();
        cases.push(("corrupt", bad));
        let mut bad = base.clone();
        let duplicate = json!({"schema":crate::generation_v2::FACT_SHARD_SCHEMA,"sequence":0,"firstKey":full["facts"][0]["factKey"],"lastKey":full["facts"][0]["factKey"],"facts":[full["facts"][0].clone(),full["facts"][0].clone()]});
        bad.shards = vec![
            store
                .put(
                    crate::generation_v2::FACT_SHARD_SCHEMA,
                    &canonical::bytes(&duplicate).unwrap(),
                )
                .unwrap(),
        ];
        cases.push(("duplicate fact", bad));
        let mut bad = base.clone();
        bad.shards[1].size += 1;
        cases.push(("size", bad));
        for (name, bad) in cases {
            let bad = manifest(&store, bad);
            let error = FactIndex::from_references(
                &store,
                &[("one", &good), ("two", &bad)],
                "analysis:java-compiler-facts",
            )
            .err()
            .expect(name);
            let d = diagnostic(&error).unwrap();
            assert_eq!(d["stage"], "ADMIT_FACT_REFERENCES", "{name}");
            assert_eq!(d["observedFacts"], 2, "{name}");
            assert!(d["plannedFacts"].is_null(), "{name}");
            assert_eq!(d["compilationOrdinal"], 2, "{name}");
        }
        // A forged manifest identity is refused independently of valid shard bytes.
        let mut bad = base;
        bad.generation_id = "sha256:forged".into();
        let bad = store
            .put(
                crate::generation_v2::GENERATION_SCHEMA,
                &canonical::bytes(&bad).unwrap(),
            )
            .unwrap();
        assert!(
            FactIndex::from_references(&store, &[("one", &bad)], "analysis:java-compiler-facts")
                .is_err()
        );
    }

    #[test]
    fn missing_and_malformed_cas_payloads_preserve_hydrated_prefix_without_completion() {
        for missing in [true, false] {
            let root = tempfile::tempdir().unwrap();
            let state = StateAuthority::open(root.path().join("state")).unwrap();
            let store = CasStore::open(&state).unwrap();
            let original = generation(
                &state,
                &store,
                &[
                    (json!({"n":1}), "analysis:java-compiler-facts"),
                    (json!({"n":2}), "analysis:java-compiler-facts"),
                ],
            );
            let lease = store.read(&original, 1024 * 1024).unwrap();
            let mut m: GenerationManifest = serde_json::from_slice(lease.bytes()).unwrap();
            let lease = store.read(&m.shards[0], 1024 * 1024).unwrap();
            let mut shard: Value = serde_json::from_slice(lease.bytes()).unwrap();
            let bad = if missing {
                CasObject::for_bytes("owned-fact/1.0", b"missing payload").unwrap()
            } else {
                store.put("owned-fact/1.0", b"malformed JSON").unwrap()
            };
            shard["facts"][1]["payload"] = serde_json::to_value(bad).unwrap();
            m.shards[0] = store
                .put(
                    crate::generation_v2::FACT_SHARD_SCHEMA,
                    &canonical::bytes(&shard).unwrap(),
                )
                .unwrap();
            let reference = manifest(&store, m);
            let error = FactIndex::from_references(
                &store,
                &[("owned", &reference)],
                "analysis:java-compiler-facts",
            )
            .err()
            .unwrap();
            assert_eq!(error.code, ErrorCode::StateCorrupt);
            let d = diagnostic(&error).unwrap();
            assert_eq!(d["stage"], "HYDRATE_FACT_PAYLOADS");
            assert_eq!(d["observedFacts"], 1);
            assert_eq!(d["plannedFacts"], 2);
            assert_eq!(d["currentRecordOrdinal"], 2);
            assert_eq!(d["currentRecordSizeVerified"], !missing);
        }
    }

    #[test]
    fn exact_record_payload_unit_accepts_below_and_at_limit() {
        for size in [MAX_RECORD_BYTES - 1, MAX_RECORD_BYTES] {
            let root = tempfile::tempdir().unwrap();
            let state = StateAuthority::open(root.path().join("state")).unwrap();
            let store = CasStore::open(&state).unwrap();
            let value = json!({"padding":"x".repeat(size as usize-14)});
            assert_eq!(serde_json::to_vec(&value).unwrap().len() as u64, size);
            let generation = generation(&state, &store, &[(value, "analysis:java-compiler-facts")]);
            let index = FactIndex::from_references(
                &store,
                &[("owned", &generation)],
                "analysis:java-compiler-facts",
            )
            .unwrap();
            assert_eq!(index.budget.observed_payload_bytes, size);
            assert!(Facts::Indexed(&index).iter().next().unwrap().is_ok());
        }
    }

    #[test]
    fn reordered_or_rebound_temporary_records_are_rejected() {
        let facts = vec![(json!({"n":1}), "a".into()), (json!({"n":2}), "b".into())];
        for update in [
            "UPDATE facts SET scope='other' WHERE ordinal=1",
            "UPDATE facts SET ordinal=3 WHERE ordinal=1",
            "UPDATE facts SET compilation=2 WHERE ordinal=1",
            "UPDATE facts SET payload=X'7B7D' WHERE ordinal=1",
        ] {
            let index = FactIndex::from_memory_for_test(&facts).unwrap();
            index.connection.execute(update, []).unwrap();
            let e = Facts::Indexed(&index).iter().next().unwrap().err().unwrap();
            assert_eq!(e.code, ErrorCode::StateCorrupt);
            assert_eq!(diagnostic(&e).unwrap()["stage"], "PROJECT_FACTS");
        }
    }

    #[test]
    fn actual_sqlite_full_preserves_typed_cause_and_measured_prefix() {
        let error = storage_refusal_fixture();
        assert_eq!(error.code, ErrorCode::ResourceLimit);
        let d = diagnostic(&error).unwrap();
        assert_eq!(d["storageCode"], 13);
        assert_eq!(d["observedFacts"], 41);
        assert_eq!(d["plannedFacts"], 100);
        assert_eq!(d["stage"], "HYDRATE_FACT_PAYLOADS");
    }
}
