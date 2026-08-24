use crate::adapter_v2::{CapabilityUri, FactRecord};
use crate::canonical;
use crate::cas::{CasObject, CasStore};
use crate::error::{ClewError, ErrorCode};
use crate::generation_v2::{GENERATION_SCHEMA, GenerationManifest};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};

pub const QUERY_INDEX_SCHEMA: &str = "codeclew-query-index/3.0";
pub const QUERY_SHARD_SCHEMA: &str = "codeclew-query-shard/2.0";
pub const QUERY_CONTEXT_SCHEMA: &str = "codeclew-query-context/2.0";
pub const MAX_QUERY_SHARD_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_QUERY_TERMS: usize = 256;
pub const MAX_CONTEXT_FACTS: usize = 4096;
/// One term must remain a bounded routing hint rather than a repository-wide
/// inverted-index dump. Overflows stay explicitly represented in the manifest
/// so query results cannot pretend that the retained prefix is complete.
pub const MAX_QUERY_FACTS_PER_TERM: usize = 1024;
/// Payload bytes recursively inspected for derived query metadata. Larger
/// payloads remain authoritative CAS evidence, but adapters must expose their
/// searchable contents as granular facts.
pub const MAX_QUERY_METADATA_PAYLOAD_BYTES: u64 = 64 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct QueryIndexManifest {
    pub schema: String,
    pub index_id: String,
    pub generation: CasObject,
    pub shards: Vec<QueryShardReference>,
    pub term_count: u64,
    pub posting_count: u64,
    pub overflow_terms: BTreeSet<String>,
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

    let (postings, overflow_terms) = bound_postings(postings);
    let term_count = postings.len() as u64;
    let posting_count = postings.values().map(|facts| facts.len() as u64).sum();
    let mut buckets = BTreeMap::<String, Vec<TermPosting>>::new();
    for (term, facts) in postings {
        buckets.entry(bucket(&term)).or_default().push(TermPosting {
            term,
            facts: facts.into_iter().collect(),
        });
    }
    let built = buckets
        .into_par_iter()
        .map(|(bucket, bucket_postings)| build_bucket(&bucket, bucket_postings))
        .collect::<Result<Vec<_>, _>>()?;
    let mut shards = built.into_iter().flatten().collect::<Vec<_>>();
    shards.sort_by(|left, right| {
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
    let mut references = publish_query_shards(store, shards)?;
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
        overflow_terms,
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
    let mut matched_by_term = Vec::with_capacity(requested_terms.len());
    let mut unmatched = Vec::new();
    let mut shards_read = BTreeSet::<String>::new();
    for term in &requested_terms {
        let bucket = bucket(term);
        let references = index.shards.iter().filter(|reference| {
            reference.bucket == bucket
                && reference.first_term.as_str() <= term.as_str()
                && term.as_str() <= reference.last_term.as_str()
        });
        let mut term_matches = BTreeSet::new();
        for reference in references {
            let lease = store.read(&reference.object, MAX_QUERY_SHARD_BYTES)?;
            shards_read.insert(reference.object.digest.clone());
            let shard: QueryShard = serde_json::from_slice(lease.bytes())
                .map_err(|_| corrupt("query shard is not a closed object"))?;
            verify_shard(reference, &shard, lease.bytes())?;
            if let Ok(position) = shard.postings.binary_search_by(|row| row.term.cmp(term)) {
                term_matches.extend(shard.postings[position].facts.iter().cloned());
            }
        }
        if term_matches.is_empty() {
            unmatched.push(term.clone());
        }
        matched_by_term.push(term_matches.into_iter().collect::<Vec<_>>());
    }
    let unique_match_count = matched_by_term
        .iter()
        .flatten()
        .cloned()
        .collect::<BTreeSet<_>>()
        .len();
    let facts = fair_fact_selection(&matched_by_term, limit);
    let truncated = unique_match_count > facts.len()
        || requested_terms
            .iter()
            .any(|term| index.overflow_terms.contains(term));
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
    let mut requested_terms = parent.requested_terms.clone();
    requested_terms.extend(normalize_terms(additional_terms.iter().map(String::as_str)));
    requested_terms.sort();
    requested_terms.dedup();
    if requested_terms == parent.requested_terms {
        return Ok(parent.clone());
    }
    query(store, index, &requested_terms, max_facts)
}

fn fair_fact_selection(matches_by_term: &[Vec<FactHit>], limit: usize) -> Vec<FactHit> {
    let diversified = matches_by_term
        .iter()
        .map(|matches| diversify_fact_families(matches))
        .collect::<Vec<_>>();
    let mut selected = BTreeSet::new();
    let mut cursors = vec![0usize; diversified.len()];
    while selected.len() < limit {
        let mut progressed = false;
        for (term_matches, cursor) in diversified.iter().zip(&mut cursors) {
            while *cursor < term_matches.len() {
                let fact = term_matches[*cursor].clone();
                *cursor += 1;
                if selected.insert(fact) {
                    progressed = true;
                    break;
                }
            }
            if selected.len() == limit {
                break;
            }
        }
        if !progressed {
            break;
        }
    }
    selected.into_iter().collect()
}

fn diversify_fact_families(matches: &[FactHit]) -> Vec<FactHit> {
    // Adapters that end opaque fact keys with a canonical digest get fair
    // representation across their stable key prefixes. Other keys fall back
    // to one deterministic family per full key, preserving prior behavior.
    let mut families = BTreeMap::<String, BTreeSet<FactHit>>::new();
    for fact in matches {
        families
            .entry(semantic_fact_key(&fact.fact_key))
            .or_default()
            .insert(fact.clone());
    }
    let families = families
        .into_values()
        .map(BTreeSet::into_iter)
        .map(Iterator::collect::<Vec<_>>)
        .collect::<Vec<_>>();
    let mut diversified = Vec::with_capacity(matches.len());
    let mut round = 0usize;
    loop {
        let before = diversified.len();
        for family in &families {
            if let Some(fact) = family.get(round) {
                diversified.push(fact.clone());
            }
        }
        if diversified.len() == before {
            break;
        }
        round += 1;
    }
    diversified
}

pub fn verify_index(store: &CasStore, index: &QueryIndexManifest) -> Result<(), ClewError> {
    verify_index_manifest(store, index)?;
    let mut previous = None;
    let mut terms = BTreeSet::new();
    let mut last_fact_by_term = BTreeMap::<String, FactHit>::new();
    let mut postings = 0u64;
    let mut postings_by_term = BTreeMap::<String, u64>::new();
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
        for posting in &shard.postings {
            if last_fact_by_term
                .get(&posting.term)
                .is_some_and(|last| last >= posting.facts.first().expect("verified non-empty"))
            {
                return Err(corrupt("query posting chunks overlap or are out of order"));
            }
            terms.insert(posting.term.clone());
            postings += posting.facts.len() as u64;
            *postings_by_term.entry(posting.term.clone()).or_default() +=
                posting.facts.len() as u64;
            last_fact_by_term.insert(
                posting.term.clone(),
                posting.facts.last().expect("verified non-empty").clone(),
            );
        }
        previous = Some(order);
    }
    if terms.len() as u64 != index.term_count
        || postings != index.posting_count
        || index.overflow_terms.iter().any(|term| {
            postings_by_term.get(term).copied() != Some(MAX_QUERY_FACTS_PER_TERM as u64)
        })
    {
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
        || index.overflow_terms.iter().any(|term| {
            term.is_empty() || normalize_terms(std::iter::once(term.as_str())) != [term.clone()]
        })
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
            reference.sequence,
        );
        if previous.is_some_and(|value| value >= order) {
            return Err(corrupt("query shard reference order is invalid"));
        }
        if reference.object.object_schema != QUERY_SHARD_SCHEMA {
            return Err(corrupt("query shard reference schema is invalid"));
        }
        if reference.object.size > MAX_QUERY_SHARD_BYTES as u64 {
            return Err(corrupt("query shard reference size is invalid"));
        }
        previous = Some(order);
    }
    Ok(())
}

fn bound_postings(
    mut postings: BTreeMap<String, BTreeSet<FactHit>>,
) -> (BTreeMap<String, BTreeSet<FactHit>>, BTreeSet<String>) {
    let mut overflow_terms = BTreeSet::new();
    for (term, facts) in &mut postings {
        if facts.len() > MAX_QUERY_FACTS_PER_TERM {
            *facts = facts
                .iter()
                .take(MAX_QUERY_FACTS_PER_TERM)
                .cloned()
                .collect();
            overflow_terms.insert(term.clone());
        }
    }
    (postings, overflow_terms)
}

#[cfg(test)]
fn publish_bucket(
    store: &CasStore,
    bucket: &str,
    postings: Vec<TermPosting>,
    references: &mut Vec<QueryShardReference>,
) -> Result<(), ClewError> {
    references.extend(publish_query_shards(
        store,
        build_bucket(bucket, postings)?,
    )?);
    Ok(())
}

fn build_bucket(bucket: &str, postings: Vec<TermPosting>) -> Result<Vec<QueryShard>, ClewError> {
    let mut current = Vec::new();
    let mut shards = Vec::new();
    let mut sequence = 0u32;
    for posting in postings {
        if canonical::bytes(&shard(bucket, sequence, std::slice::from_ref(&posting)))
            .map_err(internal)?
            .len()
            > MAX_QUERY_SHARD_BYTES
        {
            if !current.is_empty() {
                shards.push(shard(bucket, sequence, &current));
                sequence = next_sequence(sequence)?;
                current.clear();
            }
            sequence = build_split_posting(bucket, sequence, posting, &mut shards)?;
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
            shards.push(shard(bucket, sequence, &current));
            sequence = next_sequence(sequence)?;
            current = vec![last];
        }
    }
    if !current.is_empty() {
        shards.push(shard(bucket, sequence, &current));
    }
    Ok(shards)
}

fn build_split_posting(
    bucket: &str,
    mut sequence: u32,
    posting: TermPosting,
    shards: &mut Vec<QueryShard>,
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
        shards.push(shard(bucket, sequence, &[chunk]));
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

fn publish_query_shards(
    store: &CasStore,
    shards: Vec<QueryShard>,
) -> Result<Vec<QueryShardReference>, ClewError> {
    let mut encoded = Vec::with_capacity(shards.len());
    for shard in &shards {
        let bytes = canonical::bytes(shard).map_err(internal)?;
        if bytes.len() > MAX_QUERY_SHARD_BYTES {
            return Err(ClewError::new(
                ErrorCode::ResourceLimit,
                "query shard exceeds the limit",
            ));
        }
        encoded.push((QUERY_SHARD_SCHEMA.to_owned(), bytes));
    }
    let objects = store.put_batch(encoded)?;
    Ok(shards
        .into_iter()
        .zip(objects)
        .map(|(shard, object)| QueryShardReference {
            bucket: shard.bucket,
            sequence: shard.sequence,
            first_term: shard.first_term,
            last_term: shard.last_term,
            object,
        })
        .collect())
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
    let mut values = BTreeSet::from([
        semantic_fact_key(&fact.fact_key),
        fact.domain_uri.as_str().to_owned(),
    ]);
    if fact.payload.size > MAX_QUERY_METADATA_PAYLOAD_BYTES {
        return Ok(normalize_index_terms(values.iter().map(String::as_str)));
    }
    let limit = usize::try_from(fact.payload.size)
        .map_err(|_| ClewError::new(ErrorCode::ResourceLimit, "fact payload exceeds host size"))?;
    let lease = store.read(&fact.payload, limit)?;
    if let Ok(value) = serde_json::from_slice::<Value>(lease.bytes()) {
        collect_json_strings(&value, &mut values, 0)?;
    }
    Ok(normalize_index_terms(values.iter().map(String::as_str)))
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
            if !is_canonical_digest(value) {
                output.insert(value.clone());
            }
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

fn normalize_index_terms<'a>(values: impl IntoIterator<Item = &'a str>) -> Vec<String> {
    let values = values.into_iter().collect::<Vec<_>>();
    let mut terms = normalize_terms(values.iter().copied())
        .into_iter()
        .collect::<BTreeSet<_>>();
    for value in values {
        collect_identifier_aliases(value, &mut terms);
    }
    terms.into_iter().collect()
}

fn collect_identifier_aliases(value: &str, output: &mut BTreeSet<String>) {
    let mut token = String::new();
    let mut overflowed = false;
    let flush = |token: &mut String, overflowed: &mut bool, output: &mut BTreeSet<String>| {
        if !*overflowed {
            split_identifier(token, output);
        }
        token.clear();
        *overflowed = false;
    };
    for character in value.chars() {
        if character.is_alphanumeric() || character == '_' {
            if !overflowed {
                token.push(character);
                if token.len() > 256 {
                    token.clear();
                    overflowed = true;
                }
            }
        } else {
            flush(&mut token, &mut overflowed, output);
        }
    }
    flush(&mut token, &mut overflowed, output);
}

fn split_identifier(token: &str, output: &mut BTreeSet<String>) {
    for component in token.split('_') {
        if component.is_empty() {
            continue;
        }
        let characters = component.chars().collect::<Vec<_>>();
        let mut start = 0usize;
        for index in 1..characters.len() {
            let previous = characters[index - 1];
            let current = characters[index];
            let next = characters.get(index + 1).copied();
            let boundary = current.is_uppercase()
                && (previous.is_lowercase()
                    || previous.is_numeric()
                    || (previous.is_uppercase() && next.is_some_and(char::is_lowercase)));
            if boundary {
                insert_identifier_alias(&characters[start..index], output);
                start = index;
            }
        }
        insert_identifier_alias(&characters[start..], output);
    }
}

fn insert_identifier_alias(characters: &[char], output: &mut BTreeSet<String>) {
    let alias = characters
        .iter()
        .copied()
        .flat_map(char::to_lowercase)
        .collect::<String>();
    if alias.len() >= 2 && alias.len() <= 256 {
        output.insert(alias);
    }
}

fn semantic_fact_key(value: &str) -> String {
    let Some((prefix, suffix)) = value.rsplit_once(':') else {
        return value.to_owned();
    };
    if suffix.len() == 64
        && suffix
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        prefix.to_owned()
    } else {
        value.to_owned()
    }
}

fn is_canonical_digest(value: &str) -> bool {
    value.len() == 71
        && value.starts_with("sha256:")
        && value[7..]
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
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
    fn query_budget_is_shared_fairly_across_requested_terms_and_expansion() {
        let root = tempfile::tempdir().unwrap();
        let state = StateAuthority::open(root.path().join("v2")).unwrap();
        let store = CasStore::open(&state).unwrap();
        let derived = store.put(DERIVED_MANIFEST_SCHEMA, b"derived").unwrap();
        let receipt = store.put("test/receipt/1", b"complete").unwrap();
        let alpha_payload = store
            .put(
                "test/payload/1",
                br#"{"name":"Alpha","path":"src/Alpha.kt"}"#,
            )
            .unwrap();
        let beta_payload = store
            .put("test/payload/1", br#"{"name":"Beta","path":"src/Beta.kt"}"#)
            .unwrap();
        let mut writer = FactRunWriter::create(&state).unwrap();
        for index in 0..16 {
            writer
                .push(&FactRecord {
                    fact_key: format!("a:alpha:{index:02}"),
                    domain_uri: CapabilityUri::parse("analysis:symbol").unwrap(),
                    payload: alpha_payload.clone(),
                })
                .unwrap();
        }
        writer
            .push(&FactRecord {
                fact_key: "z:beta".into(),
                domain_uri: CapabilityUri::parse("analysis:symbol").unwrap(),
                payload: beta_payload,
            })
            .unwrap();
        let attempt = AttemptAuthority {
            compilation_id: "main".into(),
            capability: CapabilityUri::parse("analysis:symbol").unwrap(),
            completion: AnalysisAttemptComplete {
                scope_digest: format!("sha256:{}", "b".repeat(64)),
                completeness_receipt: receipt,
                fact_count: 17,
            },
        };
        let (generation, generation_object) = finalize_generation(
            &store,
            derived,
            vec![attempt],
            vec![writer.finish().unwrap()],
        )
        .unwrap();
        let (index, _) = build_query_index(&store, &generation, generation_object).unwrap();

        let result = query(&store, &index, &["Alpha".into(), "Beta".into()], 2).unwrap();
        assert_eq!(result.facts.len(), 2);
        assert!(
            result
                .facts
                .iter()
                .any(|fact| fact.fact_key.starts_with("a:alpha:"))
        );
        assert!(result.facts.iter().any(|fact| fact.fact_key == "z:beta"));
        assert!(result.truncated);

        let alpha = query(&store, &index, &["Alpha".into()], 2).unwrap();
        let expanded = expand(&store, &index, &alpha, &["Beta".into()], 2).unwrap();
        assert_eq!(expanded.requested_terms, vec!["alpha", "beta"]);
        assert!(
            expanded
                .facts
                .iter()
                .any(|fact| fact.fact_key.starts_with("a:alpha:"))
        );
        assert!(expanded.facts.iter().any(|fact| fact.fact_key == "z:beta"));
    }

    #[test]
    fn query_budget_is_shared_across_fact_families_within_one_term() {
        let root = tempfile::tempdir().unwrap();
        let state = StateAuthority::open(root.path().join("v2")).unwrap();
        let store = CasStore::open(&state).unwrap();
        let payload = store.put("test/payload/1", b"fact").unwrap();
        let mut matches = (0..100)
            .map(|index| FactHit {
                fact_key: format!("semantic:boundary:{index:064x}"),
                domain_uri: CapabilityUri::parse("analysis:semantic").unwrap(),
                payload: payload.clone(),
            })
            .collect::<Vec<_>>();
        matches.push(FactHit {
            fact_key: format!("semantic:descriptor:{:064x}", 200),
            domain_uri: CapabilityUri::parse("analysis:semantic").unwrap(),
            payload: payload.clone(),
        });
        matches.push(FactHit {
            fact_key: format!("semantic:relation:{:064x}", 300),
            domain_uri: CapabilityUri::parse("analysis:semantic").unwrap(),
            payload,
        });

        let selected = fair_fact_selection(&[matches.clone()], 3);
        assert_eq!(selected.len(), 3);
        assert_eq!(
            selected
                .iter()
                .map(|fact| semantic_fact_key(&fact.fact_key))
                .collect::<BTreeSet<_>>(),
            BTreeSet::from([
                "semantic:boundary".to_owned(),
                "semantic:descriptor".to_owned(),
                "semantic:relation".to_owned(),
            ])
        );
        let two = fair_fact_selection(&[matches.clone()], 2);
        assert_eq!(
            two.iter()
                .map(|fact| semantic_fact_key(&fact.fact_key))
                .collect::<BTreeSet<_>>(),
            BTreeSet::from([
                "semantic:boundary".to_owned(),
                "semantic:descriptor".to_owned(),
            ])
        );
        let one = fair_fact_selection(&[matches.clone()], 1);
        matches.reverse();
        assert_eq!(one, fair_fact_selection(&[matches], 1));
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
            overflow_terms: BTreeSet::new(),
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
    fn canonical_authority_digests_do_not_expand_semantic_term_cardinality() {
        let root = tempfile::tempdir().unwrap();
        let state = StateAuthority::open(root.path().join("v2")).unwrap();
        let store = CasStore::open(&state).unwrap();
        let first = "a".repeat(64);
        let second = "b".repeat(64);
        let facts = [
            FactRecord {
                fact_key: format!("kotlin:descriptor:{first}"),
                domain_uri: CapabilityUri::parse("analysis:kotlin-semantic-facts").unwrap(),
                payload: store
                    .put(
                        "test/payload/1",
                        format!(r#"{{"name":"sha256","rawRowHash":"sha256:{first}"}}"#).as_bytes(),
                    )
                    .unwrap(),
            },
            FactRecord {
                fact_key: format!("kotlin:descriptor:{second}"),
                domain_uri: CapabilityUri::parse("analysis:kotlin-semantic-facts").unwrap(),
                payload: store
                    .put(
                        "test/payload/1",
                        format!(r#"{{"name":"sha256","rawRowHash":"sha256:{second}"}}"#).as_bytes(),
                    )
                    .unwrap(),
            },
        ];

        let terms = facts
            .iter()
            .flat_map(|fact| terms_for_fact(&store, fact).unwrap())
            .collect::<BTreeSet<_>>();
        assert_eq!(
            terms,
            BTreeSet::from([
                "analysis".to_owned(),
                "descriptor".to_owned(),
                "facts".to_owned(),
                "kotlin".to_owned(),
                "semantic".to_owned(),
                "sha256".to_owned(),
            ])
        );
        assert!(!terms.contains(&first));
        assert!(!terms.contains(&second));
    }

    #[test]
    fn oversized_payloads_require_granular_query_facts() {
        let root = tempfile::tempdir().unwrap();
        let state = StateAuthority::open(root.path().join("v2")).unwrap();
        let store = CasStore::open(&state).unwrap();
        let exact_prefix = br#"{"name":"BoundarySymbol","padding":""#;
        let suffix = br#""}"#;
        let padding = MAX_QUERY_METADATA_PAYLOAD_BYTES as usize - exact_prefix.len() - suffix.len();
        let mut exact_bytes = exact_prefix.to_vec();
        exact_bytes.extend(std::iter::repeat_n(b'x', padding));
        exact_bytes.extend_from_slice(suffix);
        assert_eq!(exact_bytes.len(), MAX_QUERY_METADATA_PAYLOAD_BYTES as usize);
        let exact = FactRecord {
            fact_key: "symbol:boundary".into(),
            domain_uri: CapabilityUri::parse("analysis:semantic").unwrap(),
            payload: store.put("test/payload/1", &exact_bytes).unwrap(),
        };
        assert!(
            terms_for_fact(&store, &exact)
                .unwrap()
                .contains(&"boundarysymbol".to_owned())
        );

        let oversized_json = serde_json::json!({
            "declarations": [{
                "name": "UniqueNestedSymbol",
                "padding": "x".repeat(MAX_QUERY_METADATA_PAYLOAD_BYTES as usize)
            }]
        });
        let oversized = FactRecord {
            fact_key: "file:monolith".into(),
            domain_uri: CapabilityUri::parse("analysis:semantic").unwrap(),
            payload: store
                .put(
                    "test/payload/1",
                    &serde_json::to_vec(&oversized_json).unwrap(),
                )
                .unwrap(),
        };
        let oversized_terms = terms_for_fact(&store, &oversized).unwrap();
        assert_eq!(
            oversized_terms,
            vec![
                "analysis".to_owned(),
                "file".to_owned(),
                "monolith".to_owned(),
                "semantic".to_owned(),
            ]
        );

        let alternate_oversized = FactRecord {
            payload: store
                .put(
                    "test/payload/1",
                    &serde_json::to_vec(&serde_json::json!({
                        "different": vec!["CompletelyDifferent"; 10_000]
                    }))
                    .unwrap(),
                )
                .unwrap(),
            ..oversized.clone()
        };
        assert!(alternate_oversized.payload.size > MAX_QUERY_METADATA_PAYLOAD_BYTES);
        assert_eq!(
            terms_for_fact(&store, &alternate_oversized).unwrap(),
            oversized_terms
        );

        let granular = FactRecord {
            fact_key: "symbol:unique".into(),
            domain_uri: CapabilityUri::parse("analysis:semantic").unwrap(),
            payload: store
                .put(
                    "test/payload/1",
                    br#"{"name":"UniqueNestedSymbol","path":"src/Unique.kt"}"#,
                )
                .unwrap(),
        };
        assert!(
            !terms_for_fact(&store, &oversized)
                .unwrap()
                .contains(&"uniquenestedsymbol".to_owned())
        );
        assert!(
            terms_for_fact(&store, &granular)
                .unwrap()
                .contains(&"uniquenestedsymbol".to_owned())
        );

        let derived = store.put(DERIVED_MANIFEST_SCHEMA, b"derived").unwrap();
        let receipt = store.put("test/receipt/1", b"complete").unwrap();
        let mut writer = FactRunWriter::create(&state).unwrap();
        writer.push(&oversized).unwrap();
        writer.push(&granular).unwrap();
        let attempt = AttemptAuthority {
            compilation_id: "main".into(),
            capability: CapabilityUri::parse("analysis:semantic").unwrap(),
            completion: AnalysisAttemptComplete {
                scope_digest: format!("sha256:{}", "c".repeat(64)),
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
        let (index, _) = build_query_index(&store, &generation, generation_object).unwrap();
        let symbol = query(&store, &index, &["UniqueNestedSymbol".into()], 10).unwrap();
        assert_eq!(symbol.facts.len(), 1);
        assert_eq!(symbol.facts[0].fact_key, "symbol:unique");
        let category = query(&store, &index, &["monolith".into()], 10).unwrap();
        assert_eq!(category.facts.len(), 1);
        assert_eq!(category.facts[0].fact_key, "file:monolith");
    }

    #[test]
    fn index_identifier_aliases_enable_natural_queries_without_expanding_query_terms() {
        assert_eq!(
            normalize_terms(["MavenProjectModel"]),
            vec!["mavenprojectmodel"]
        );
        assert_eq!(
            normalize_index_terms(["MavenProjectModel", "XMLHttpRequest", "load_project_state"]),
            vec![
                "http",
                "load",
                "load_project_state",
                "maven",
                "mavenprojectmodel",
                "model",
                "project",
                "request",
                "state",
                "xml",
                "xmlhttprequest",
            ]
        );
        assert!(!normalize_index_terms(["A_b"]).contains(&"a".to_owned()));

        let root = tempfile::tempdir().unwrap();
        let state = StateAuthority::open(root.path().join("v2")).unwrap();
        let store = CasStore::open(&state).unwrap();
        let derived = store.put(DERIVED_MANIFEST_SCHEMA, b"derived").unwrap();
        let receipt = store.put("test/receipt/1", b"complete").unwrap();
        let relevant = FactRecord {
            fact_key: "a:symbol:relevant".into(),
            domain_uri: CapabilityUri::parse("analysis:symbol").unwrap(),
            payload: store
                .put(
                    "test/payload/1",
                    br#"{"name":"MavenProjectModel","operation":"load_project_state"}"#,
                )
                .unwrap(),
        };
        let noise = FactRecord {
            fact_key: "b:symbol:noise".into(),
            domain_uri: CapabilityUri::parse("analysis:symbol").unwrap(),
            payload: store
                .put("test/payload/1", br#"{"name":"UnrelatedProject"}"#)
                .unwrap(),
        };
        let mut writer = FactRunWriter::create(&state).unwrap();
        writer.push(&relevant).unwrap();
        writer.push(&noise).unwrap();
        let attempt = AttemptAuthority {
            compilation_id: "main".into(),
            capability: CapabilityUri::parse("analysis:symbol").unwrap(),
            completion: AnalysisAttemptComplete {
                scope_digest: format!("sha256:{}", "d".repeat(64)),
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
        let (index, _) = build_query_index(&store, &generation, generation_object).unwrap();
        for term in ["Maven", "Project", "load", "state", "MavenProjectModel"] {
            let result = query(&store, &index, &[term.into()], 10).unwrap();
            assert!(
                result
                    .facts
                    .iter()
                    .any(|fact| fact.fact_key == "a:symbol:relevant"),
                "natural term {term} must find the identifier fact"
            );
        }
        let focused = query(&store, &index, &["Maven".into(), "Project".into()], 1).unwrap();
        assert_eq!(focused.facts.len(), 1);
        assert_eq!(focused.facts[0].fact_key, "a:symbol:relevant");
    }

    #[test]
    fn file_summary_does_not_starve_a_distinctive_descriptor() {
        let root = tempfile::tempdir().unwrap();
        let state = StateAuthority::open(root.path().join("v2")).unwrap();
        let store = CasStore::open(&state).unwrap();
        let derived = store.put(DERIVED_MANIFEST_SCHEMA, b"derived").unwrap();
        let receipt = store.put("test/receipt/1", b"complete").unwrap();
        let file_summary = FactRecord {
            fact_key: "a:kotlin:file:large".into(),
            domain_uri: CapabilityUri::parse("analysis:kotlin-semantic-facts").unwrap(),
            payload: store
                .put(
                    "codeclew-kotlin-semantic-fact/3.0",
                    br#"{"path":"src/Large.kt","semanticFactCount":100000,"semanticFactsDigest":"sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}"#,
                )
                .unwrap(),
        };
        let descriptor = FactRecord {
            fact_key: "b:kotlin:descriptor:useful".into(),
            domain_uri: CapabilityUri::parse("analysis:kotlin-semantic-facts").unwrap(),
            payload: store
                .put(
                    "codeclew-kotlin-semantic-fact/3.0",
                    br#"{"compilerCallableId":"pkg/UsefulDescriptor.find","symbolIdentity":"callable:pkg/UsefulDescriptor.find"}"#,
                )
                .unwrap(),
        };
        let mut writer = FactRunWriter::create(&state).unwrap();
        writer.push(&file_summary).unwrap();
        writer.push(&descriptor).unwrap();
        let attempt = AttemptAuthority {
            compilation_id: "main".into(),
            capability: CapabilityUri::parse("analysis:kotlin-semantic-facts").unwrap(),
            completion: AnalysisAttemptComplete {
                scope_digest: format!("sha256:{}", "e".repeat(64)),
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
        let (index, _) = build_query_index(&store, &generation, generation_object).unwrap();

        let result = query(&store, &index, &["UsefulDescriptor".into()], 1).unwrap();
        assert_eq!(result.facts.len(), 1);
        assert_eq!(result.facts[0].fact_key, descriptor.fact_key);
        assert!(!result.truncated);
        let payload = store.read(&result.facts[0].payload, 4096).unwrap();
        let text = String::from_utf8_lossy(payload.bytes());
        assert!(text.contains("UsefulDescriptor"));
        assert!(!text.contains("semanticFacts"));
        assert!(!text.contains("OpaqueSiblingMarker"));

        let dropped = query(&store, &index, &["OpaqueSiblingMarker".into()], 1).unwrap();
        assert!(dropped.facts.is_empty());
        assert!(!dropped.truncated);
    }

    #[test]
    fn high_fanout_term_is_bounded_and_reported_as_truncated() {
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
        let (bounded, overflow_terms) = bound_postings(BTreeMap::from([(
            "popular".to_owned(),
            facts.iter().cloned().collect(),
        )]));
        let retained = bounded["popular"].iter().cloned().collect::<Vec<_>>();
        let posting = TermPosting {
            term: "popular".into(),
            facts: retained.clone(),
        };
        let posting_bucket = bucket("popular");
        let mut references = Vec::new();
        publish_bucket(&store, &posting_bucket, vec![posting], &mut references).unwrap();
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
        assert_eq!(recovered, retained);
        assert_eq!(recovered.len(), MAX_QUERY_FACTS_PER_TERM);
        let mut index = QueryIndexManifest {
            schema: QUERY_INDEX_SCHEMA.into(),
            index_id: String::new(),
            generation: store.put(GENERATION_SCHEMA, b"generation").unwrap(),
            shards: references,
            term_count: 1,
            posting_count: retained.len() as u64,
            overflow_terms,
        };
        index.index_id = canonical::hash(&index).unwrap();
        verify_index(&store, &index).unwrap();
        let result = query(&store, &index, &["popular".into()], MAX_CONTEXT_FACTS).unwrap();
        assert_eq!(result.facts.len(), MAX_QUERY_FACTS_PER_TERM);
        assert!(result.truncated);
    }
}
