use crate::adapter_v2::{CapabilityUri, FactRecord};
use crate::canonical;
use crate::cas::{CasObject, CasStore};
use crate::error::{ClewError, ErrorCode};
use crate::generation_v2::{GENERATION_SCHEMA, GenerationManifest};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};

pub const QUERY_INDEX_SCHEMA: &str = "codeclew-query-index/2.0";
pub const QUERY_SHARD_SCHEMA: &str = "codeclew-query-shard/2.0";
pub const QUERY_CONTEXT_SCHEMA: &str = "codeclew-query-context/2.0";
pub const MAX_QUERY_SHARD_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_QUERY_TERMS: usize = 256;
pub const MAX_CONTEXT_FACTS: usize = 4096;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct QueryIndexManifest {
    pub schema: String,
    pub index_id: String,
    pub generation: CasObject,
    pub shards: Vec<QueryShardReference>,
    pub term_count: u64,
    pub posting_count: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct QueryShardReference {
    pub bucket: String,
    pub sequence: u32,
    pub first_term: String,
    pub last_term: String,
    pub object: CasObject,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct QueryShard {
    schema: String,
    bucket: String,
    sequence: u32,
    first_term: String,
    last_term: String,
    postings: Vec<TermPosting>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct TermPosting {
    term: String,
    facts: Vec<FactHit>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FactHit {
    pub fact_key: String,
    pub domain_uri: CapabilityUri,
    pub payload: CasObject,
}

impl Ord for FactHit {
    fn cmp(&self, other: &Self) -> Ordering {
        (
            self.fact_key.as_str(),
            self.domain_uri.as_str(),
            self.payload.object_schema.as_str(),
            self.payload.digest.as_str(),
            self.payload.size,
        )
            .cmp(&(
                other.fact_key.as_str(),
                other.domain_uri.as_str(),
                other.payload.object_schema.as_str(),
                other.payload.digest.as_str(),
                other.payload.size,
            ))
    }
}

impl PartialOrd for FactHit {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct QueryContext {
    pub schema: String,
    pub index_id: String,
    pub requested_terms: Vec<String>,
    pub unmatched_terms: Vec<String>,
    pub facts: Vec<FactHit>,
    pub query_shards_read: u32,
    pub truncated: bool,
}

pub fn build_query_index(
    store: &CasStore,
    generation: &GenerationManifest,
    generation_object: CasObject,
) -> Result<(QueryIndexManifest, CasObject), ClewError> {
    if generation_object.object_schema != GENERATION_SCHEMA {
        return Err(invalid("query index requires a generation manifest object"));
    }
    let mut postings = BTreeMap::<String, BTreeSet<FactHit>>::new();
    generation.visit_facts(store, |fact| {
        let hit = FactHit {
            fact_key: fact.fact_key.clone(),
            domain_uri: fact.domain_uri.clone(),
            payload: fact.payload.clone(),
        };
        let mut terms = terms_for_fact(store, fact)?;
        terms.sort();
        terms.dedup();
        for term in terms {
            postings.entry(term).or_default().insert(hit.clone());
        }
        Ok(())
    })?;
    if postings.is_empty() {
        return Err(invalid("generation produced no queryable terms"));
    }

    let term_count = postings.len() as u64;
    let posting_count = postings.values().map(|facts| facts.len() as u64).sum();
    let mut buckets = BTreeMap::<String, Vec<TermPosting>>::new();
    for (term, facts) in postings {
        buckets.entry(bucket(&term)).or_default().push(TermPosting {
            term,
            facts: facts.into_iter().collect(),
        });
    }
    let mut references = Vec::new();
    for (bucket, bucket_postings) in buckets {
        publish_bucket(store, &bucket, bucket_postings, &mut references)?;
    }
    references.sort_by(|left, right| {
        (
            &left.bucket,
            &left.first_term,
            &left.last_term,
            left.sequence,
        )
            .cmp(&(
                &right.bucket,
                &right.first_term,
                &right.last_term,
                right.sequence,
            ))
    });
    let mut manifest = QueryIndexManifest {
        schema: QUERY_INDEX_SCHEMA.into(),
        index_id: String::new(),
        generation: generation_object,
        shards: references,
        term_count,
        posting_count,
    };
    manifest.index_id = canonical::hash(&manifest).map_err(internal)?;
    let object = store.put(
        QUERY_INDEX_SCHEMA,
        &canonical::bytes(&manifest).map_err(internal)?,
    )?;
    Ok((manifest, object))
}

pub fn query(
    store: &CasStore,
    index: &QueryIndexManifest,
    terms: &[String],
    max_facts: usize,
) -> Result<QueryContext, ClewError> {
    verify_index_manifest(store, index)?;
    if terms.is_empty() || terms.len() > MAX_QUERY_TERMS || max_facts == 0 {
        return Err(invalid("query term or result limit is invalid"));
    }
    let limit = max_facts.min(MAX_CONTEXT_FACTS);
    let requested_terms = normalize_terms(terms.iter().map(String::as_str));
    if requested_terms.is_empty() {
        return Err(invalid("query has no normalized terms"));
    }
    let mut matched = BTreeSet::new();
    let mut unmatched = Vec::new();
    let mut shards_read = BTreeSet::<String>::new();
    for term in &requested_terms {
        let bucket = bucket(term);
        let references = index.shards.iter().filter(|reference| {
            reference.bucket == bucket
                && reference.first_term.as_str() <= term.as_str()
                && term.as_str() <= reference.last_term.as_str()
        });
        let mut found = false;
        for reference in references {
            let lease = store.read(&reference.object, MAX_QUERY_SHARD_BYTES)?;
            shards_read.insert(reference.object.digest.clone());
            let shard: QueryShard = serde_json::from_slice(lease.bytes())
                .map_err(|_| corrupt("query shard is not a closed object"))?;
            verify_shard(reference, &shard, lease.bytes())?;
            if let Ok(position) = shard.postings.binary_search_by(|row| row.term.cmp(term)) {
                found = true;
                matched.extend(shard.postings[position].facts.iter().cloned());
            }
        }
        if !found {
            unmatched.push(term.clone());
        }
    }
    let truncated = matched.len() > limit;
    let facts = matched.into_iter().take(limit).collect();
    Ok(QueryContext {
        schema: QUERY_CONTEXT_SCHEMA.into(),
        index_id: index.index_id.clone(),
        requested_terms,
        unmatched_terms: unmatched,
        facts,
        query_shards_read: shards_read.len() as u32,
        truncated,
    })
}

pub fn expand(
    store: &CasStore,
    index: &QueryIndexManifest,
    parent: &QueryContext,
    additional_terms: &[String],
    max_facts: usize,
) -> Result<QueryContext, ClewError> {
    if parent.schema != QUERY_CONTEXT_SCHEMA || parent.index_id != index.index_id {
        return Err(invalid("parent context is not bound to the query index"));
    }
    let additional = normalize_terms(additional_terms.iter().map(String::as_str))
        .into_iter()
        .filter(|term| !parent.requested_terms.contains(term))
        .collect::<Vec<_>>();
    if additional.is_empty() {
        return Ok(parent.clone());
    }
    let delta = query(store, index, &additional, max_facts)?;
    let mut requested_terms = parent.requested_terms.clone();
    requested_terms.extend(delta.requested_terms);
    requested_terms.sort();
    requested_terms.dedup();
    let mut unmatched_terms = parent.unmatched_terms.clone();
    unmatched_terms.extend(delta.unmatched_terms);
    unmatched_terms.sort();
    unmatched_terms.dedup();
    let mut facts = parent.facts.clone();
    facts.extend(delta.facts);
    facts.sort();
    facts.dedup();
    let limit = max_facts.min(MAX_CONTEXT_FACTS);
    let truncated = parent.truncated || delta.truncated || facts.len() > limit;
    facts.truncate(limit);
    Ok(QueryContext {
        schema: QUERY_CONTEXT_SCHEMA.into(),
        index_id: index.index_id.clone(),
        requested_terms,
        unmatched_terms,
        facts,
        query_shards_read: delta.query_shards_read,
        truncated,
    })
}

pub fn verify_index(store: &CasStore, index: &QueryIndexManifest) -> Result<(), ClewError> {
    verify_index_manifest(store, index)?;
    let mut previous = None;
    let mut terms = 0u64;
    let mut postings = 0u64;
    for reference in &index.shards {
        let order = (
            reference.bucket.as_str(),
            reference.first_term.as_str(),
            reference.last_term.as_str(),
            reference.sequence,
        );
        if previous.is_some_and(|value| value >= order) {
            return Err(corrupt("query shard reference order is invalid"));
        }
        let lease = store.read(&reference.object, MAX_QUERY_SHARD_BYTES)?;
        let shard: QueryShard = serde_json::from_slice(lease.bytes())
            .map_err(|_| corrupt("query shard is not a closed object"))?;
        verify_shard(reference, &shard, lease.bytes())?;
        terms += shard.postings.len() as u64;
        postings += shard
            .postings
            .iter()
            .map(|posting| posting.facts.len() as u64)
            .sum::<u64>();
        previous = Some(order);
    }
    if terms != index.term_count || postings != index.posting_count {
        return Err(corrupt("query index counts are incomplete"));
    }
    Ok(())
}

pub fn verify_index_manifest(
    store: &CasStore,
    index: &QueryIndexManifest,
) -> Result<(), ClewError> {
    let mut unsigned = index.clone();
    unsigned.index_id.clear();
    if index.schema != QUERY_INDEX_SCHEMA
        || index.generation.object_schema != GENERATION_SCHEMA
        || index.index_id != canonical::hash(&unsigned).map_err(internal)?
        || index.shards.is_empty()
        || index.term_count == 0
        || index.posting_count == 0
    {
        return Err(corrupt("query index identity is invalid"));
    }
    store.read(
        &index.generation,
        usize::try_from(index.generation.size)
            .map_err(|_| invalid("generation manifest exceeds host size"))?,
    )?;
    let mut previous = None;
    for reference in &index.shards {
        let order = (
            reference.bucket.as_str(),
            reference.first_term.as_str(),
            reference.last_term.as_str(),
        );
        if previous.is_some_and(|value| value >= order)
            || reference.object.object_schema != QUERY_SHARD_SCHEMA
            || reference.object.size > MAX_QUERY_SHARD_BYTES as u64
        {
            return Err(corrupt("query shard reference order is invalid"));
        }
        previous = Some(order);
    }
    Ok(())
}

fn publish_bucket(
    store: &CasStore,
    bucket: &str,
    postings: Vec<TermPosting>,
    references: &mut Vec<QueryShardReference>,
) -> Result<(), ClewError> {
    let mut current = Vec::new();
    let mut sequence = 0u32;
    for posting in postings {
        if canonical::bytes(&shard(bucket, sequence, std::slice::from_ref(&posting)))
            .map_err(internal)?
            .len()
            > MAX_QUERY_SHARD_BYTES
        {
            if !current.is_empty() {
                publish_query_shard(store, shard(bucket, sequence, &current), references)?;
                sequence = next_sequence(sequence)?;
                current.clear();
            }
            sequence = publish_split_posting(store, bucket, sequence, posting, references)?;
            continue;
        }
        current.push(posting);
        let candidate = shard(bucket, sequence, &current);
        let bytes = canonical::bytes(&candidate).map_err(internal)?;
        if bytes.len() > MAX_QUERY_SHARD_BYTES {
            let last = current.pop().expect("candidate contains one posting");
            if current.is_empty() {
                return Err(ClewError::new(
                    ErrorCode::ResourceLimit,
                    "one query posting exceeds the shard limit",
                ));
            }
            publish_query_shard(store, shard(bucket, sequence, &current), references)?;
            sequence = next_sequence(sequence)?;
            current = vec![last];
        }
    }
    if !current.is_empty() {
        publish_query_shard(store, shard(bucket, sequence, &current), references)?;
    }
    Ok(())
}

fn publish_split_posting(
    store: &CasStore,
    bucket: &str,
    mut sequence: u32,
    posting: TermPosting,
    references: &mut Vec<QueryShardReference>,
) -> Result<u32, ClewError> {
    let mut start = 0usize;
    while start < posting.facts.len() {
        let remaining = posting.facts.len() - start;
        let mut low = 1usize;
        let mut high = remaining;
        let mut fitting = 0usize;
        while low <= high {
            let middle = low + (high - low) / 2;
            let candidate = TermPosting {
                term: posting.term.clone(),
                facts: posting.facts[start..start + middle].to_vec(),
            };
            let size = canonical::bytes(&shard(bucket, sequence, &[candidate]))
                .map_err(internal)?
                .len();
            if size <= MAX_QUERY_SHARD_BYTES {
                fitting = middle;
                low = middle + 1;
            } else {
                high = middle.saturating_sub(1);
            }
        }
        if fitting == 0 {
            return Err(ClewError::new(
                ErrorCode::ResourceLimit,
                "one query fact reference exceeds the shard limit",
            ));
        }
        let chunk = TermPosting {
            term: posting.term.clone(),
            facts: posting.facts[start..start + fitting].to_vec(),
        };
        publish_query_shard(store, shard(bucket, sequence, &[chunk]), references)?;
        sequence = next_sequence(sequence)?;
        start += fitting;
    }
    Ok(sequence)
}

fn next_sequence(sequence: u32) -> Result<u32, ClewError> {
    sequence
        .checked_add(1)
        .ok_or_else(|| invalid("query shard sequence overflow"))
}

fn shard(bucket: &str, sequence: u32, postings: &[TermPosting]) -> QueryShard {
    QueryShard {
        schema: QUERY_SHARD_SCHEMA.into(),
        bucket: bucket.into(),
        sequence,
        first_term: postings.first().expect("non-empty postings").term.clone(),
        last_term: postings.last().expect("non-empty postings").term.clone(),
        postings: postings.to_vec(),
    }
}

fn publish_query_shard(
    store: &CasStore,
    shard: QueryShard,
    references: &mut Vec<QueryShardReference>,
) -> Result<(), ClewError> {
    let bytes = canonical::bytes(&shard).map_err(internal)?;
    if bytes.len() > MAX_QUERY_SHARD_BYTES {
        return Err(ClewError::new(
            ErrorCode::ResourceLimit,
            "query shard exceeds the limit",
        ));
    }
    let object = store.put(QUERY_SHARD_SCHEMA, &bytes)?;
    references.push(QueryShardReference {
        bucket: shard.bucket,
        sequence: shard.sequence,
        first_term: shard.first_term,
        last_term: shard.last_term,
        object,
    });
    Ok(())
}

fn verify_shard(
    reference: &QueryShardReference,
    shard: &QueryShard,
    bytes: &[u8],
) -> Result<(), ClewError> {
    if canonical::bytes(shard).map_err(internal)? != bytes
        || shard.schema != QUERY_SHARD_SCHEMA
        || shard.bucket != reference.bucket
        || shard.sequence != reference.sequence
        || shard.first_term != reference.first_term
        || shard.last_term != reference.last_term
        || shard.postings.is_empty()
        || shard.first_term != shard.postings.first().expect("non-empty").term
        || shard.last_term != shard.postings.last().expect("non-empty").term
        || !shard
            .postings
            .windows(2)
            .all(|pair| pair[0].term < pair[1].term)
        || shard.postings.iter().any(|posting| {
            posting.term.is_empty()
                || bucket(&posting.term) != shard.bucket
                || posting.facts.is_empty()
                || !posting.facts.windows(2).all(|pair| pair[0] < pair[1])
        })
    {
        return Err(corrupt("query shard authority is invalid"));
    }
    Ok(())
}

fn terms_for_fact(store: &CasStore, fact: &FactRecord) -> Result<Vec<String>, ClewError> {
    let mut values = BTreeSet::from([fact.fact_key.clone(), fact.domain_uri.as_str().to_owned()]);
    let limit = usize::try_from(fact.payload.size)
        .map_err(|_| ClewError::new(ErrorCode::ResourceLimit, "fact payload exceeds host size"))?;
    let lease = store.read(&fact.payload, limit)?;
    if let Ok(value) = serde_json::from_slice::<Value>(lease.bytes()) {
        collect_json_strings(&value, &mut values, 0)?;
    }
    Ok(normalize_terms(values.iter().map(String::as_str)))
}

fn collect_json_strings(
    value: &Value,
    output: &mut BTreeSet<String>,
    depth: usize,
) -> Result<(), ClewError> {
    if depth > 64 || output.len() > 65_536 {
        return Err(ClewError::new(
            ErrorCode::ResourceLimit,
            "fact payload query metadata exceeds limits",
        ));
    }
    match value {
        Value::String(value) => {
            output.insert(value.clone());
        }
        Value::Array(values) => {
            for value in values {
                collect_json_strings(value, output, depth + 1)?;
            }
        }
        Value::Object(values) => {
            for value in values.values() {
                collect_json_strings(value, output, depth + 1)?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn normalize_terms<'a>(values: impl IntoIterator<Item = &'a str>) -> Vec<String> {
    let mut terms = BTreeSet::new();
    for value in values {
        let mut current = String::new();
        for character in value.chars().flat_map(char::to_lowercase) {
            if character.is_alphanumeric() || character == '_' {
                current.push(character);
                if current.len() > 256 {
                    current.clear();
                }
            } else if !current.is_empty() {
                if current.len() >= 2 {
                    terms.insert(std::mem::take(&mut current));
                } else {
                    current.clear();
                }
            }
        }
        if current.len() >= 2 {
            terms.insert(current);
        }
    }
    terms.into_iter().collect()
}

fn bucket(term: &str) -> String {
    let digest = Sha256::digest(term.as_bytes());
    // A one-byte routing prefix caps cold publication at 256 durability
    // boundaries. Oversized buckets are still split by MAX_QUERY_SHARD_BYTES,
    // while exact queries continue to read only their routed bucket.
    hex::encode(&digest[..1])
}

fn invalid(message: &str) -> ClewError {
    ClewError::new(ErrorCode::InvalidInput, message)
}

fn corrupt(message: &str) -> ClewError {
    ClewError::new(ErrorCode::StateCorrupt, message)
}

fn internal(error: impl std::fmt::Display) -> ClewError {
    ClewError::new(ErrorCode::Internal, error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapter_v2::{AnalysisAttemptComplete, CapabilityUri};
    use crate::derived_manifest::DERIVED_MANIFEST_SCHEMA;
    use crate::generation_v2::{AttemptAuthority, FactRunWriter, finalize_generation};
    use crate::state::StateAuthority;

    #[test]
    fn index_is_deterministic_and_query_reads_only_term_buckets() {
        let root = tempfile::tempdir().unwrap();
        let state = StateAuthority::open(root.path().join("v2")).unwrap();
        let store = CasStore::open(&state).unwrap();
        let derived = store.put(DERIVED_MANIFEST_SCHEMA, b"derived").unwrap();
        let receipt = store.put("test/receipt/1", b"complete").unwrap();
        let payload_a = store
            .put(
                "test/payload/1",
                br#"{"name":"AlphaService","path":"src/A.kt"}"#,
            )
            .unwrap();
        let payload_b = store
            .put(
                "test/payload/1",
                br#"{"name":"BetaService","path":"src/B.kt"}"#,
            )
            .unwrap();
        let facts = [
            FactRecord {
                fact_key: "symbol:alpha".into(),
                domain_uri: CapabilityUri::parse("analysis:symbol").unwrap(),
                payload: payload_a,
            },
            FactRecord {
                fact_key: "symbol:beta".into(),
                domain_uri: CapabilityUri::parse("analysis:symbol").unwrap(),
                payload: payload_b,
            },
        ];
        let mut writer = FactRunWriter::create(&state).unwrap();
        for fact in &facts {
            writer.push(fact).unwrap();
        }
        let attempt = AttemptAuthority {
            compilation_id: "main".into(),
            capability: CapabilityUri::parse("analysis:symbol").unwrap(),
            completion: AnalysisAttemptComplete {
                scope_digest: format!("sha256:{}", "a".repeat(64)),
                completeness_receipt: receipt,
                fact_count: 2,
            },
        };
        let (generation, generation_object) = finalize_generation(
            &store,
            derived,
            vec![attempt],
            vec![writer.finish().unwrap()],
        )
        .unwrap();
        let (first, first_object) =
            build_query_index(&store, &generation, generation_object.clone()).unwrap();
        let (second, second_object) =
            build_query_index(&store, &generation, generation_object).unwrap();
        assert_eq!(first, second);
        assert_eq!(first_object, second_object);
        let result = query(&store, &first, &["AlphaService".into()], 10).unwrap();
        assert_eq!(result.facts.len(), 1);
        assert_eq!(result.facts[0].fact_key, "symbol:alpha");
        assert_eq!(result.query_shards_read, 1);
        let expanded = expand(&store, &first, &result, &["BetaService".into()], 10).unwrap();
        assert_eq!(expanded.facts.len(), 2);
        assert!(expanded.query_shards_read <= 2);
        assert_eq!(
            first
                .shards
                .iter()
                .map(|shard| &shard.bucket)
                .collect::<BTreeSet<_>>()
                .len(),
            first.shards.len()
        );
        assert!(first.shards.len() <= 256);
    }

    #[test]
    fn tampered_or_cross_index_expansion_fails_closed() {
        let parent = QueryContext {
            schema: QUERY_CONTEXT_SCHEMA.into(),
            index_id: "sha256:old".into(),
            requested_terms: vec!["alpha".into()],
            unmatched_terms: vec![],
            facts: vec![],
            query_shards_read: 0,
            truncated: false,
        };
        let root = tempfile::tempdir().unwrap();
        let state = StateAuthority::open(root.path().join("v2")).unwrap();
        let store = CasStore::open(&state).unwrap();
        let index = QueryIndexManifest {
            schema: QUERY_INDEX_SCHEMA.into(),
            index_id: "sha256:new".into(),
            generation: store.put(GENERATION_SCHEMA, b"generation").unwrap(),
            shards: vec![],
            term_count: 0,
            posting_count: 0,
        };
        assert_eq!(
            expand(&store, &index, &parent, &["beta".into()], 10)
                .unwrap_err()
                .code,
            ErrorCode::InvalidInput
        );
    }

    #[test]
    fn repeated_payload_metadata_is_deduplicated_before_the_bound() {
        let repeated = Value::Array(
            (0..70_000)
                .map(|_| serde_json::json!({"kind":"declaration","name":"Target"}))
                .collect(),
        );
        let mut strings = BTreeSet::new();
        collect_json_strings(&repeated, &mut strings, 0).unwrap();
        assert_eq!(
            strings,
            BTreeSet::from(["Target".to_owned(), "declaration".to_owned()])
        );
    }

    #[test]
    fn high_fanout_term_is_split_without_dropping_fact_references() {
        let root = tempfile::tempdir().unwrap();
        let state = StateAuthority::open(root.path().join("v2")).unwrap();
        let store = CasStore::open(&state).unwrap();
        let payload = store.put("test/payload/1", b"x").unwrap();
        let facts = (0..50_000)
            .map(|index| FactHit {
                fact_key: format!("symbol:{index:08}"),
                domain_uri: CapabilityUri::parse("analysis:symbol").unwrap(),
                payload: payload.clone(),
            })
            .collect::<Vec<_>>();
        let posting = TermPosting {
            term: "popular".into(),
            facts: facts.clone(),
        };
        let posting_bucket = bucket("popular");
        assert!(
            canonical::bytes(&shard(&posting_bucket, 0, std::slice::from_ref(&posting)))
                .unwrap()
                .len()
                > MAX_QUERY_SHARD_BYTES
        );
        let mut references = Vec::new();
        publish_bucket(&store, &posting_bucket, vec![posting], &mut references).unwrap();
        assert!(references.len() > 1);
        assert!(
            references
                .windows(2)
                .all(|pair| pair[0].sequence < pair[1].sequence)
        );
        let recovered = references
            .iter()
            .flat_map(|reference| {
                let lease = store
                    .read(&reference.object, MAX_QUERY_SHARD_BYTES)
                    .unwrap();
                let shard: QueryShard = serde_json::from_slice(lease.bytes()).unwrap();
                verify_shard(reference, &shard, lease.bytes()).unwrap();
                shard.postings.into_iter().flat_map(|posting| posting.facts)
            })
            .collect::<Vec<_>>();
        assert_eq!(recovered, facts);
    }
}
