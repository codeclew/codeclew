//! Pure construction and lookup for the thread-owned Kotlin callable index.
//!
//! This module deliberately has no filesystem, process, session-lifecycle, or
//! publication behavior.  Its input is the result of the closed Kotlin
//! semantic validators plus exact member/source/CAS authority.  Its output is
//! a set of canonical bytes and the CAS identities those bytes will acquire.
//! A caller may therefore preflight every bound result before publishing any
//! object or retaining a thread root.

use crate::canonical;
use crate::cas::{CAS_OBJECT_SCHEMA, CasObject};
use crate::error::{ClewError, ErrorCode};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path};

#[cfg(test)]
thread_local! {
    static FACT_SHARD_SERIALIZATIONS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static QUERY_SHARD_SERIALIZATIONS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

pub const CALLABLE_FACT_SCHEMA: &str = "codeclew-kotlin-callable-fact/1.0";
pub const CALLABLE_FACT_SHARD_SCHEMA: &str = "codeclew-kotlin-callable-fact-shard/1.0";
pub const CALLABLE_QUERY_SHARD_SCHEMA: &str = "codeclew-kotlin-callable-query-shard/1.0";
pub const CALLABLE_QUERY_INDEX_SCHEMA: &str = "codeclew-kotlin-callable-query-index/1.0";
pub const CALLABLE_FACT_SET_SCHEMA: &str = "codeclew-kotlin-callable-fact-set/1.0";
pub const CALLABLE_FACT_SET_EVIDENCE_SCHEMA: &str =
    "codeclew-kotlin-callable-fact-set-evidence/1.0";
pub const CALLABLE_FACT_SET_PROJECTION_SCHEMA: &str =
    "codeclew-kotlin-callable-fact-set-projection/1.0";
pub const KOTLIN_SEMANTIC_FACT_SCHEMA: &str = "codeclew-kotlin-semantic-fact/3.0";

pub const MAX_CALLABLE_MEMBERS: usize = 8;
pub const MAX_CALLABLE_PAIR_BINDINGS: usize = 32;
pub const MAX_CALLABLE_COMPILATIONS: usize = 64;
pub const MAX_INPUT_FACTS_VISITED: usize = 131_072;
pub const MAX_INPUT_PAYLOAD_BYTES: usize = 32 * 1024 * 1024;
pub const MAX_DECLARATION_FACTS: usize = 65_536;
pub const MAX_USE_FACTS: usize = 65_536;
pub const MAX_BOUNDARY_FACTS: usize = 16_384;
pub const MAX_NORMALIZED_FACTS: usize = 131_072;
pub const MAX_PARAMETERS_PER_CALLABLE: usize = 1_024;
pub const MAX_TYPE_PARAMETERS: usize = 256;
pub const MAX_BOUNDS_PER_TYPE_PARAMETER: usize = 64;
pub const MAX_CONTAINMENT_DEPTH: usize = 64;
pub const MAX_CALLABLE_TEXT_BYTES: usize = 4_096;
pub const MAX_CALLABLE_SHARD_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_DERIVED_CAS_OBJECTS: usize = 64;
pub const MAX_CALLABLE_EVIDENCE_OBJECT_BYTES: usize = 64 * 1024 * 1024;
pub const MAX_DIRECT_CAS_CLOSURE_BYTES: usize = 96 * 1024 * 1024;
pub const MAX_SELECTED_SOURCE_BYTES: usize = 64 * 1024 * 1024;
pub const MAX_CALLABLE_QUERY_TERMS: usize = 256;
pub const MAX_CALLABLE_QUERY_RESULTS: usize = 4_096;
pub const MAX_CALLABLE_STDOUT_BYTES: usize = 64 * 1024;
const MAX_INDEX_TERMS_PER_FACT: usize = 65_536;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CallableBudgets {
    pub max_members: usize,
    pub max_pair_bindings: usize,
    pub max_compilations: usize,
    pub max_input_facts_visited: usize,
    pub max_input_payload_bytes: usize,
    pub max_declarations: usize,
    pub max_uses: usize,
    pub max_boundaries: usize,
    pub max_normalized_facts: usize,
    pub max_parameters_per_callable: usize,
    pub max_type_parameters: usize,
    pub max_bounds_per_type_parameter: usize,
    pub max_containment_depth: usize,
    pub max_text_bytes: usize,
    pub max_shard_bytes: usize,
    pub max_derived_cas_objects: usize,
    pub max_direct_cas_closure_bytes: usize,
    pub max_query_terms: usize,
    pub max_query_results: usize,
    pub max_stdout_bytes: usize,
}

impl CallableBudgets {
    pub fn frozen() -> Self {
        Self {
            max_members: MAX_CALLABLE_MEMBERS,
            max_pair_bindings: MAX_CALLABLE_PAIR_BINDINGS,
            max_compilations: MAX_CALLABLE_COMPILATIONS,
            max_input_facts_visited: MAX_INPUT_FACTS_VISITED,
            max_input_payload_bytes: MAX_INPUT_PAYLOAD_BYTES,
            max_declarations: MAX_DECLARATION_FACTS,
            max_uses: MAX_USE_FACTS,
            max_boundaries: MAX_BOUNDARY_FACTS,
            max_normalized_facts: MAX_NORMALIZED_FACTS,
            max_parameters_per_callable: MAX_PARAMETERS_PER_CALLABLE,
            max_type_parameters: MAX_TYPE_PARAMETERS,
            max_bounds_per_type_parameter: MAX_BOUNDS_PER_TYPE_PARAMETER,
            max_containment_depth: MAX_CONTAINMENT_DEPTH,
            max_text_bytes: MAX_CALLABLE_TEXT_BYTES,
            max_shard_bytes: MAX_CALLABLE_SHARD_BYTES,
            max_derived_cas_objects: MAX_DERIVED_CAS_OBJECTS,
            max_direct_cas_closure_bytes: MAX_DIRECT_CAS_CLOSURE_BYTES,
            max_query_terms: MAX_CALLABLE_QUERY_TERMS,
            max_query_results: MAX_CALLABLE_QUERY_RESULTS,
            max_stdout_bytes: MAX_CALLABLE_STDOUT_BYTES,
        }
    }

    fn validate(&self) -> Result<(), ClewError> {
        if self != &Self::frozen() {
            return Err(invalid(
                "Kotlin callable budgets differ from the frozen profile",
            ));
        }
        Ok(())
    }
}

impl Default for CallableBudgets {
    fn default() -> Self {
        Self::frozen()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum GraphCoverage {
    CompleteSupportedSubset,
    Partial,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum RelationshipAuthority {
    VerifiedSameSnapshotCompilationDependency,
    DeclaredTopology,
    Unbound,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CallableTaskBinding {
    pub task_id: String,
    pub pair_id: String,
    pub terms: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CallablePairBinding {
    pub pair_id: String,
    pub provider_member: String,
    pub consumer_member: String,
    pub relationship_authority: RelationshipAuthority,
    pub dependency_evidence_ref: Option<CasObject>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CallableFactSetRequest {
    pub thread_id: String,
    pub thread_authority_digest: String,
    pub thread_context_id: String,
    pub thread_context_authority_digest: String,
    pub profile_digest: String,
    pub tasks: Vec<CallableTaskBinding>,
    pub pairs: Vec<CallablePairBinding>,
    pub budgets: CallableBudgets,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CallableMemberAuthority {
    pub member_alias: String,
    pub service_alias: String,
    pub session_id: String,
    pub session_authority_digest: String,
    pub repository_key: String,
    pub base_revision: String,
    pub snapshot_ref: CasObject,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CallableCompilationAuthority {
    pub compilation_id: String,
    pub generation_id: String,
    pub generation_ref: CasObject,
    pub semantic_authority: String,
    pub extractor_id: String,
    pub adapter_digest: String,
    pub runtime_digest: String,
    pub descriptor_coverage: GraphCoverage,
    pub relation_coverage: GraphCoverage,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct QualifiedCallablePayload {
    pub member: CallableMemberAuthority,
    pub compilation: CallableCompilationAuthority,
    pub fact_key: String,
    pub payload_ref: CasObject,
    pub source_ref: Option<CasObject>,
    pub payload: Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CallableSelectedCompilation {
    pub member: CallableMemberAuthority,
    pub compilation: CallableCompilationAuthority,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CallableBuildInput {
    pub visited_fact_count: usize,
    pub visited_payload_bytes: usize,
    pub selected_compilations: Vec<CallableSelectedCompilation>,
    pub payloads: Vec<QualifiedCallablePayload>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CallableCompilationBinding {
    pub compilation_id: String,
    pub generation_id: String,
    pub generation_ref: CasObject,
    pub semantic_authority: String,
    pub extractor_id: String,
    pub adapter_digest: String,
    pub runtime_digest: String,
    pub descriptor_coverage: GraphCoverage,
    pub relation_coverage: GraphCoverage,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CallableMemberBinding {
    pub member_alias: String,
    pub service_alias: String,
    pub session_id: String,
    pub session_authority_digest: String,
    pub repository_key: String,
    pub repository_namespace: String,
    pub base_revision: String,
    pub snapshot_ref: CasObject,
    pub compilations: Vec<CallableCompilationBinding>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SourceAnchor {
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end: Option<u64>,
    pub content_ref: CasObject,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CallableFactProvenance {
    pub member_alias: String,
    pub repository_namespace: String,
    pub session_id: String,
    pub session_authority_digest: String,
    pub base_revision: String,
    pub compilation_id: String,
    pub generation_id: String,
    pub generation_ref: CasObject,
    pub input_fact_key: String,
    pub input_payload_ref: CasObject,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<SourceAnchor>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum DeclarationKind {
    Function,
    Constructor,
    Class,
    Property,
    MutableProperty,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DeclarationFact {
    pub schema: String,
    pub fact_id: String,
    pub provenance: CallableFactProvenance,
    pub declaration_kind: DeclarationKind,
    pub symbol_identity: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub compiler_callable_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub compiler_class_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub jvm_descriptor: Option<String>,
    pub owner_identity: String,
    pub containment: Vec<String>,
    pub projected_shape: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shape_digest: Option<String>,
    pub exact_eligible: bool,
    pub uncertainty_reasons: Vec<String>,
}

impl Eq for DeclarationFact {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum TargetResolution {
    ExactSymbol,
    CallableFamily,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UseFact {
    pub schema: String,
    pub fact_id: String,
    pub provenance: CallableFactProvenance,
    pub relation_kind: String,
    pub source_owner: String,
    pub target_callable_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_symbol_identity: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_repository_namespace: Option<String>,
    pub target_resolution: TargetResolution,
    pub relationship_authority: RelationshipAuthority,
    pub relation_evidence: Value,
    pub exact_eligible: bool,
    pub uncertainty_reasons: Vec<String>,
}

impl Eq for UseFact {}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BoundaryFact {
    pub schema: String,
    pub fact_id: String,
    pub provenance: CallableFactProvenance,
    pub stage: String,
    pub code: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subject: Option<String>,
    pub required_checks: Vec<String>,
    pub boundary_evidence: Value,
}

impl Eq for BoundaryFact {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "rowKind",
    content = "row",
    rename_all = "SCREAMING_SNAKE_CASE",
    deny_unknown_fields
)]
pub enum CallableFact {
    Declaration(DeclarationFact),
    Use(UseFact),
    Boundary(BoundaryFact),
}

impl CallableFact {
    pub fn fact_id(&self) -> &str {
        match self {
            Self::Declaration(row) => &row.fact_id,
            Self::Use(row) => &row.fact_id,
            Self::Boundary(row) => &row.fact_id,
        }
    }

    fn provenance(&self) -> &CallableFactProvenance {
        match self {
            Self::Declaration(row) => &row.provenance,
            Self::Use(row) => &row.provenance,
            Self::Boundary(row) => &row.provenance,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CallableFactCounts {
    pub visited_input_facts: usize,
    pub visited_input_payload_bytes: usize,
    pub declarations: usize,
    pub uses: usize,
    pub boundaries: usize,
    pub total: usize,
    pub exact_declarations: usize,
    pub exact_uses: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum CallableFactSetCoverage {
    Complete,
    Partial,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum CallableFactSetCertainty {
    Verified,
    Unsure,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CallableCompleteness {
    pub coverage: CallableFactSetCoverage,
    pub certainty: CallableFactSetCertainty,
    pub obligation_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CallableFactShardReference {
    pub sequence: u32,
    pub first_fact_id: String,
    pub last_fact_id: String,
    pub object: CasObject,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CallableFactShard {
    pub schema: String,
    pub sequence: u32,
    pub first_fact_id: String,
    pub last_fact_id: String,
    pub facts: Vec<CallableFact>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum CallableLookupKind {
    FullSymbol,
    CallableFamily,
    Token,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CallableIndexKey {
    pub kind: CallableLookupKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repository_namespace: Option<String>,
    pub value: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum CallableFactKind {
    Declaration,
    Use,
    Boundary,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CallablePosting {
    pub fact_id: String,
    pub fact_kind: CallableFactKind,
    pub member_alias: String,
    pub repository_namespace: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub symbol_identity: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub callable_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_start: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_end: Option<u64>,
    pub exact_eligible: bool,
    pub uncertainty_reasons: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CallableIndexRecord {
    pub key: CallableIndexKey,
    pub posting: CallablePosting,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CallableQueryShard {
    pub schema: String,
    pub binding_digest: String,
    pub sequence: u32,
    pub first_key: CallableIndexKey,
    pub last_key: CallableIndexKey,
    pub records: Vec<CallableIndexRecord>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CallableQueryShardReference {
    pub sequence: u32,
    pub first_key: CallableIndexKey,
    pub last_key: CallableIndexKey,
    pub object: CasObject,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CallableQueryIndexManifest {
    pub schema: String,
    pub index_id: String,
    pub binding_digest: String,
    pub fact_shards: Vec<CallableFactShardReference>,
    pub shards: Vec<CallableQueryShardReference>,
    pub exact_symbol_key_count: usize,
    pub callable_family_key_count: usize,
    pub token_key_count: usize,
    pub posting_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CallableFactSetEvidence {
    pub schema: String,
    pub binding_digest: String,
    pub counts: CallableFactCounts,
    pub completeness: CallableCompleteness,
    pub members: Vec<CallableMemberBinding>,
    pub fact_shards: Vec<CallableFactShardReference>,
    pub query_index_ref: CasObject,
    pub boundary_fact_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CallableFactSetAuthority {
    pub schema: String,
    pub authority_digest: String,
    pub binding_digest: String,
    pub thread_id: String,
    pub thread_authority_digest: String,
    pub thread_context_id: String,
    pub thread_context_authority_digest: String,
    pub profile_digest: String,
    pub tasks: Vec<CallableTaskBinding>,
    pub pairs: Vec<CallablePairBinding>,
    pub budgets: CallableBudgets,
    pub members: Vec<CallableMemberBinding>,
    pub counts: CallableFactCounts,
    pub completeness: CallableCompleteness,
    pub fact_shards: Vec<CallableFactShardReference>,
    pub query_index_ref: CasObject,
    pub evidence_ref: CasObject,
    pub direct_cas_closure: Vec<CasObject>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CallableProjectionMember {
    pub member_alias: String,
    pub service_alias: String,
    pub repository_namespace: String,
    pub compilations: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CallableProjectionTask {
    pub task_id: String,
    pub pair_id: String,
    pub term_count: usize,
    pub terms_digest: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CallableFactSetProjection {
    pub schema: String,
    pub fact_set_id: String,
    pub authority_digest: String,
    pub binding_digest: String,
    pub thread_id: String,
    pub thread_context_id: String,
    pub tasks: Vec<CallableProjectionTask>,
    pub pairs: Vec<CallablePairBinding>,
    pub members: Vec<CallableProjectionMember>,
    pub counts: CallableFactCounts,
    pub completeness: CallableCompleteness,
    pub query_index_ref: CasObject,
    pub evidence_ref: CasObject,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedCasObject {
    pub reference: CasObject,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedCallableFactSet {
    pub authority: CallableFactSetAuthority,
    pub evidence: CallableFactSetEvidence,
    pub projection: CallableFactSetProjection,
    pub fact_shards: Vec<PreparedCasObject>,
    pub query_shards: Vec<PreparedCasObject>,
    pub query_index: CallableQueryIndexManifest,
    pub query_index_object: PreparedCasObject,
    pub evidence_object: PreparedCasObject,
    pub authority_bytes: Vec<u8>,
    pub projection_bytes: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "SCREAMING_SNAKE_CASE", deny_unknown_fields)]
pub enum CallableLookup {
    FullSymbol {
        repository_namespace: String,
        symbol_identity: String,
    },
    CallableFamily {
        #[serde(skip_serializing_if = "Option::is_none")]
        repository_namespace: Option<String>,
        callable_id: String,
    },
    Token {
        term: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CallableQueryRequest {
    pub lookups: Vec<CallableLookup>,
    pub max_results: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum LookupAuthority {
    ExactFullSymbol,
    NavigationOnly,
    Unsure,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum CallableQueryStatus {
    ExactFullSymbol,
    NavigationOnly,
    Ambiguous,
    Unsure,
    NotFound,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CallableQueryHit {
    pub lookup: CallableLookup,
    pub posting: CallablePosting,
    pub authority: LookupAuthority,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CallableQueryResult {
    pub schema: String,
    pub index_id: String,
    pub requested_lookups: Vec<CallableLookup>,
    pub unmatched_lookups: Vec<CallableLookup>,
    pub hits: Vec<CallableQueryHit>,
    pub status: CallableQueryStatus,
    pub truncated: bool,
    pub query_shards_read: usize,
    pub verification_obligations: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CallableQueryTrace {
    pub result: CallableQueryResult,
    pub matched_postings: Vec<CallablePosting>,
    pub query_shard_refs: Vec<CasObject>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
struct CallableBindingMaterial<'a> {
    schema: &'static str,
    request: &'a CallableFactSetRequest,
    input_visit: InputVisitBinding,
    members: &'a [CallableMemberBinding],
    payloads: Vec<InputPayloadBinding>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
struct InputVisitBinding {
    fact_count: usize,
    payload_bytes: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
struct InputPayloadBinding {
    member_alias: String,
    compilation_id: String,
    fact_key: String,
    payload_ref: CasObject,
    source_ref: Option<CasObject>,
}

/// Derive a path-free namespace that remains stable for one managed repository
/// and cannot collide merely because two repositories expose the same Kotlin
/// CallableId.
pub fn repository_namespace(repository_key: &str) -> Result<String, ClewError> {
    validate_text(repository_key, "repository key")?;
    if repository_key.contains('/')
        || repository_key.contains('\\')
        || repository_key.contains("://")
    {
        return Err(invalid(
            "repository key is not a path-free managed identity",
        ));
    }
    let digest = canonical::hash(&json!({
        "schema":"codeclew-repository-namespace/1.0",
        "repositoryKey":repository_key,
    }))
    .map_err(internal)?;
    Ok(format!(
        "repo:{}",
        digest.strip_prefix("sha256:").unwrap_or(&digest)
    ))
}

pub fn build(
    request: CallableFactSetRequest,
    input: CallableBuildInput,
) -> Result<PreparedCallableFactSet, ClewError> {
    build_with_jobs(request, input, 1)
}

/// The construction job count is operational only.  It is deliberately not
/// present in binding material, and every job count follows the same sorted
/// pure construction path.
pub fn build_with_jobs(
    mut request: CallableFactSetRequest,
    mut input: CallableBuildInput,
    jobs: usize,
) -> Result<PreparedCallableFactSet, ClewError> {
    if !(1..=64).contains(&jobs) {
        return Err(invalid(
            "callable construction jobs must be between one and 64",
        ));
    }
    request.budgets.validate()?;
    normalize_request(&mut request)?;
    if input.selected_compilations.is_empty()
        || input.selected_compilations.len() > request.budgets.max_compilations
    {
        return Err(budget(
            "callable selected compilations are empty or exceed 64",
        ));
    }
    if input.payloads.is_empty() {
        return Err(invalid(
            "callable construction has no qualified Kotlin payloads",
        ));
    }
    if input.visited_fact_count < input.payloads.len()
        || input.visited_fact_count > request.budgets.max_input_facts_visited
        || input.visited_payload_bytes > request.budgets.max_input_payload_bytes
    {
        return Err(budget("callable input visit budget is invalid or exceeded"));
    }

    for selected in &input.selected_compilations {
        validate_selected_compilation(selected)?;
    }
    input.selected_compilations.sort_by(|left, right| {
        (
            left.member.member_alias.as_str(),
            left.compilation.compilation_id.as_str(),
            left.compilation.generation_id.as_str(),
        )
            .cmp(&(
                right.member.member_alias.as_str(),
                right.compilation.compilation_id.as_str(),
                right.compilation.generation_id.as_str(),
            ))
    });
    if input.selected_compilations.windows(2).any(|pair| {
        pair[0].member.member_alias == pair[1].member.member_alias
            && pair[0].compilation.compilation_id == pair[1].compilation.compilation_id
    }) {
        return Err(invalid(
            "selected Kotlin compilation identity is duplicated",
        ));
    }
    let members = member_bindings(&input.selected_compilations, &request.budgets)?;
    validate_pair_members(&request, &members)?;
    let selected_lookup = input
        .selected_compilations
        .iter()
        .map(|selected| {
            (
                (
                    selected.member.member_alias.as_str(),
                    selected.compilation.compilation_id.as_str(),
                ),
                selected,
            )
        })
        .collect::<BTreeMap<_, _>>();

    let mut actual_payload_bytes = 0usize;
    for payload in &input.payloads {
        let selected = selected_lookup
            .get(&(
                payload.member.member_alias.as_str(),
                payload.compilation.compilation_id.as_str(),
            ))
            .ok_or_else(|| invalid("Kotlin payload references an unselected compilation"))?;
        if selected.member != payload.member || selected.compilation != payload.compilation {
            return Err(invalid(
                "Kotlin payload substituted selected member or compilation authority",
            ));
        }
        let bytes = validate_qualified_payload(payload, &request.budgets)?;
        actual_payload_bytes = actual_payload_bytes
            .checked_add(bytes)
            .ok_or_else(|| budget("callable payload byte count overflowed"))?;
    }
    if input.visited_payload_bytes < actual_payload_bytes {
        return Err(invalid(
            "visited payload bytes omit selected semantic payloads",
        ));
    }

    input.payloads.sort_by(|left, right| {
        (
            left.member.member_alias.as_str(),
            left.compilation.compilation_id.as_str(),
            left.fact_key.as_str(),
            left.payload_ref.digest.as_str(),
        )
            .cmp(&(
                right.member.member_alias.as_str(),
                right.compilation.compilation_id.as_str(),
                right.fact_key.as_str(),
                right.payload_ref.digest.as_str(),
            ))
    });
    if input.payloads.windows(2).any(|pair| {
        pair[0].member.member_alias == pair[1].member.member_alias
            && pair[0].compilation.compilation_id == pair[1].compilation.compilation_id
            && pair[0].fact_key == pair[1].fact_key
    }) {
        return Err(invalid(
            "qualified Kotlin input repeats a generation fact identity",
        ));
    }

    let payload_bindings = input
        .payloads
        .iter()
        .map(|payload| InputPayloadBinding {
            member_alias: payload.member.member_alias.clone(),
            compilation_id: payload.compilation.compilation_id.clone(),
            fact_key: payload.fact_key.clone(),
            payload_ref: payload.payload_ref.clone(),
            source_ref: payload.source_ref.clone(),
        })
        .collect::<Vec<_>>();
    let binding_digest = canonical::hash(&CallableBindingMaterial {
        schema: "codeclew-kotlin-callable-binding/1.0",
        request: &request,
        input_visit: InputVisitBinding {
            fact_count: input.visited_fact_count,
            payload_bytes: input.visited_payload_bytes,
        },
        members: &members,
        payloads: payload_bindings,
    })
    .map_err(internal)?;

    let declaration_inputs = input
        .payloads
        .iter()
        .filter(|payload| payload_schema(payload) == Some("declaration-descriptor/0.1"))
        .collect::<Vec<_>>();
    let use_inputs = input
        .payloads
        .iter()
        .filter(|payload| payload_schema(payload) == Some("declaration-relation/0.1"))
        .collect::<Vec<_>>();
    let boundary_inputs = input
        .payloads
        .iter()
        .filter(|payload| {
            matches!(
                payload_schema(payload),
                Some("declaration-descriptor-boundary/0.1" | "declaration-relation-boundary/0.1")
            )
        })
        .collect::<Vec<_>>();
    validate_fact_counts(
        &request.budgets,
        declaration_inputs.len(),
        use_inputs.len(),
        boundary_inputs.len(),
    )?;

    let mut boundaries = boundary_inputs
        .into_iter()
        .map(|payload| normalize_boundary(payload, &request.budgets))
        .collect::<Result<Vec<_>, _>>()?;
    boundaries.sort_by(|left, right| left.fact_id.cmp(&right.fact_id));
    if boundaries
        .windows(2)
        .any(|pair| pair[0].fact_id == pair[1].fact_id)
    {
        return Err(invalid("normalized Kotlin boundaries repeat an identity"));
    }

    let mut declarations = declaration_inputs
        .into_iter()
        .map(|payload| normalize_declaration(payload, &boundaries, &request.budgets))
        .collect::<Result<Vec<_>, _>>()?;
    declarations.sort_by(|left, right| left.fact_id.cmp(&right.fact_id));
    reject_duplicate_declarations(&declarations)?;
    let declaration_lookup = declarations
        .iter()
        .map(|row| {
            (
                (
                    row.provenance.repository_namespace.clone(),
                    row.symbol_identity.clone(),
                ),
                row,
            )
        })
        .collect::<BTreeMap<_, _>>();
    let member_namespaces = members
        .iter()
        .map(|member| {
            (
                member.member_alias.clone(),
                member.repository_namespace.clone(),
            )
        })
        .collect::<BTreeMap<_, _>>();

    let mut uses = use_inputs
        .into_iter()
        .map(|payload| {
            normalize_use(
                payload,
                &request,
                &declaration_lookup,
                &member_namespaces,
                &boundaries,
                &request.budgets,
            )
        })
        .collect::<Result<Vec<_>, _>>()?;
    uses.sort_by(|left, right| left.fact_id.cmp(&right.fact_id));
    if uses
        .windows(2)
        .any(|pair| pair[0].fact_id == pair[1].fact_id)
    {
        return Err(invalid("normalized Kotlin uses repeat an identity"));
    }
    let counts = CallableFactCounts {
        visited_input_facts: input.visited_fact_count,
        visited_input_payload_bytes: input.visited_payload_bytes,
        declarations: declarations.len(),
        uses: uses.len(),
        boundaries: boundaries.len(),
        total: declarations.len() + uses.len() + boundaries.len(),
        exact_declarations: declarations.iter().filter(|row| row.exact_eligible).count(),
        exact_uses: uses.iter().filter(|row| row.exact_eligible).count(),
    };
    let has_partial_compilation =
        members
            .iter()
            .flat_map(|member| &member.compilations)
            .any(|compilation| {
                compilation.descriptor_coverage == GraphCoverage::Partial
                    || compilation.relation_coverage == GraphCoverage::Partial
            });
    let completeness = CallableCompleteness {
        coverage: if boundaries.is_empty() && !has_partial_compilation {
            CallableFactSetCoverage::Complete
        } else {
            CallableFactSetCoverage::Partial
        },
        certainty: if boundaries.is_empty() && !has_partial_compilation {
            CallableFactSetCertainty::Verified
        } else {
            CallableFactSetCertainty::Unsure
        },
        obligation_count: boundaries.len(),
    };

    let mut facts = declarations
        .into_iter()
        .map(CallableFact::Declaration)
        .chain(uses.into_iter().map(CallableFact::Use))
        .chain(boundaries.into_iter().map(CallableFact::Boundary))
        .collect::<Vec<_>>();
    facts.sort_by(|left, right| left.fact_id().cmp(right.fact_id()));
    if facts
        .windows(2)
        .any(|pair| pair[0].fact_id() >= pair[1].fact_id())
    {
        return Err(invalid("normalized callable facts are not unique"));
    }

    let (fact_shards, fact_shard_refs) = build_fact_shards(&facts, &request.budgets)?;
    let index_records = build_index_records(&facts)?;
    let (query_shards, query_shard_refs) =
        build_query_shards(&binding_digest, &index_records, &request.budgets)?;
    let key_counts = index_key_counts(&index_records);
    let mut query_index = CallableQueryIndexManifest {
        schema: CALLABLE_QUERY_INDEX_SCHEMA.into(),
        index_id: String::new(),
        binding_digest: binding_digest.clone(),
        fact_shards: fact_shard_refs.clone(),
        shards: query_shard_refs,
        exact_symbol_key_count: key_counts.0,
        callable_family_key_count: key_counts.1,
        token_key_count: key_counts.2,
        posting_count: index_records.len(),
    };
    query_index.index_id = query_index_id(&query_index)?;
    let query_index_bytes = canonical::bytes(&query_index).map_err(internal)?;
    let query_index_ref = CasObject::for_bytes(CALLABLE_QUERY_INDEX_SCHEMA, &query_index_bytes)?;
    let query_index_object = PreparedCasObject {
        reference: query_index_ref.clone(),
        bytes: query_index_bytes,
    };

    let boundary_fact_ids = facts
        .iter()
        .filter_map(|fact| match fact {
            CallableFact::Boundary(row) => Some(row.fact_id.clone()),
            _ => None,
        })
        .collect::<Vec<_>>();
    let evidence = CallableFactSetEvidence {
        schema: CALLABLE_FACT_SET_EVIDENCE_SCHEMA.into(),
        binding_digest: binding_digest.clone(),
        counts: counts.clone(),
        completeness: completeness.clone(),
        members: members.clone(),
        fact_shards: fact_shard_refs.clone(),
        query_index_ref: query_index_ref.clone(),
        boundary_fact_ids,
    };
    let evidence_bytes = canonical::bytes(&evidence).map_err(internal)?;
    let evidence_ref = CasObject::for_bytes(CALLABLE_FACT_SET_EVIDENCE_SCHEMA, &evidence_bytes)?;
    let evidence_object = PreparedCasObject {
        reference: evidence_ref.clone(),
        bytes: evidence_bytes,
    };

    let mut direct_refs = Vec::new();
    for selected in &input.selected_compilations {
        direct_refs.extend([
            selected.member.snapshot_ref.clone(),
            selected.compilation.generation_ref.clone(),
        ]);
    }
    for payload in &input.payloads {
        direct_refs.push(payload.payload_ref.clone());
        if let Some(source) = &payload.source_ref {
            direct_refs.push(source.clone());
        }
    }
    for pair in &request.pairs {
        if let Some(reference) = &pair.dependency_evidence_ref {
            direct_refs.push(reference.clone());
        }
    }
    direct_refs.extend(fact_shards.iter().map(|object| object.reference.clone()));
    direct_refs.extend(query_shards.iter().map(|object| object.reference.clone()));
    direct_refs.push(query_index_ref.clone());
    direct_refs.push(evidence_ref.clone());
    let direct_cas_closure = canonical_cas_closure(direct_refs, &request.budgets)?;
    let derived_count = fact_shards.len() + query_shards.len() + 2;
    if derived_count > request.budgets.max_derived_cas_objects {
        return Err(budget("callable derived CAS object count exceeds 64"));
    }

    let mut authority = CallableFactSetAuthority {
        schema: CALLABLE_FACT_SET_SCHEMA.into(),
        authority_digest: String::new(),
        binding_digest: binding_digest.clone(),
        thread_id: request.thread_id.clone(),
        thread_authority_digest: request.thread_authority_digest.clone(),
        thread_context_id: request.thread_context_id.clone(),
        thread_context_authority_digest: request.thread_context_authority_digest.clone(),
        profile_digest: request.profile_digest.clone(),
        tasks: request.tasks.clone(),
        pairs: request.pairs.clone(),
        budgets: request.budgets.clone(),
        members: members.clone(),
        counts: counts.clone(),
        completeness: completeness.clone(),
        fact_shards: fact_shard_refs,
        query_index_ref: query_index_ref.clone(),
        evidence_ref: evidence_ref.clone(),
        direct_cas_closure,
    };
    authority.authority_digest = authority_digest(&authority)?;
    let authority_bytes = canonical::bytes(&authority).map_err(internal)?;
    let projection = projection_from_authority(&authority);
    let projection_bytes = canonical::bytes(&projection).map_err(internal)?;
    if projection_bytes.len().saturating_add(1) > request.budgets.max_stdout_bytes {
        return Err(budget("callable projection plus LF exceeds 64 KiB"));
    }
    let prepared = PreparedCallableFactSet {
        authority,
        evidence,
        projection,
        fact_shards,
        query_shards,
        query_index,
        query_index_object,
        evidence_object,
        authority_bytes,
        projection_bytes,
    };
    verify_prepared(&prepared)?;
    Ok(prepared)
}

fn projection_from_authority(authority: &CallableFactSetAuthority) -> CallableFactSetProjection {
    CallableFactSetProjection {
        schema: CALLABLE_FACT_SET_PROJECTION_SCHEMA.into(),
        fact_set_id: format!("thread-callables:{}", authority.authority_digest),
        authority_digest: authority.authority_digest.clone(),
        binding_digest: authority.binding_digest.clone(),
        thread_id: authority.thread_id.clone(),
        thread_context_id: authority.thread_context_id.clone(),
        tasks: authority
            .tasks
            .iter()
            .map(|task| CallableProjectionTask {
                task_id: task.task_id.clone(),
                pair_id: task.pair_id.clone(),
                term_count: task.terms.len(),
                terms_digest: canonical::hash(&task.terms)
                    .expect("validated task terms are canonically serializable"),
            })
            .collect(),
        pairs: authority.pairs.clone(),
        members: authority
            .members
            .iter()
            .map(|member| CallableProjectionMember {
                member_alias: member.member_alias.clone(),
                service_alias: member.service_alias.clone(),
                repository_namespace: member.repository_namespace.clone(),
                compilations: member
                    .compilations
                    .iter()
                    .map(|compilation| compilation.compilation_id.clone())
                    .collect(),
            })
            .collect(),
        counts: authority.counts.clone(),
        completeness: authority.completeness.clone(),
        query_index_ref: authority.query_index_ref.clone(),
        evidence_ref: authority.evidence_ref.clone(),
    }
}

pub fn verify_authority_projection(
    authority: &CallableFactSetAuthority,
    projection: &CallableFactSetProjection,
) -> Result<(), ClewError> {
    authority.budgets.validate()?;
    validate_authority_compilation_bindings(authority)?;
    if authority_digest(authority)? != authority.authority_digest
        || projection != &projection_from_authority(authority)
    {
        return Err(corrupt(
            "callable authority or its compact projection was substituted",
        ));
    }
    Ok(())
}

fn validate_authority_compilation_bindings(
    authority: &CallableFactSetAuthority,
) -> Result<(), ClewError> {
    let mut total = 0usize;
    let mut previous_member = None::<&str>;
    for member in &authority.members {
        if previous_member.is_some_and(|previous| previous >= member.member_alias.as_str())
            || member.compilations.is_empty()
        {
            return Err(corrupt(
                "callable authority member/compilation order is invalid",
            ));
        }
        previous_member = Some(&member.member_alias);
        let mut previous_compilation = None::<&str>;
        for compilation in &member.compilations {
            if previous_compilation
                .is_some_and(|previous| previous >= compilation.compilation_id.as_str())
            {
                return Err(corrupt(
                    "callable authority compilation bindings repeat or are unordered",
                ));
            }
            previous_compilation = Some(&compilation.compilation_id);
            total = total
                .checked_add(1)
                .ok_or_else(|| corrupt("callable authority compilation count overflowed"))?;
        }
    }
    if total == 0 || total > authority.budgets.max_compilations {
        return Err(corrupt(
            "callable authority selected compilation count exceeds 64",
        ));
    }
    Ok(())
}

fn normalize_request(request: &mut CallableFactSetRequest) -> Result<(), ClewError> {
    validate_text(&request.thread_id, "thread id")?;
    validate_digest(&request.thread_authority_digest, "thread authority digest")?;
    validate_text(&request.thread_context_id, "thread context id")?;
    validate_digest(
        &request.thread_context_authority_digest,
        "thread context authority digest",
    )?;
    validate_digest(&request.profile_digest, "callable profile digest")?;
    if request.tasks.is_empty()
        || request.pairs.is_empty()
        || request.pairs.len() > request.budgets.max_pair_bindings
    {
        return Err(invalid(
            "callable task and pair bindings are empty or exceed bounds",
        ));
    }

    for pair in &request.pairs {
        validate_alias(&pair.pair_id, "pair id")?;
        validate_alias(&pair.provider_member, "provider member")?;
        validate_alias(&pair.consumer_member, "consumer member")?;
        if pair.provider_member == pair.consumer_member {
            return Err(invalid(
                "callable pair cannot substitute one member for both roles",
            ));
        }
        match (
            pair.relationship_authority,
            pair.dependency_evidence_ref.as_ref(),
        ) {
            (RelationshipAuthority::VerifiedSameSnapshotCompilationDependency, Some(reference)) => {
                validate_cas_object(reference)?;
            }
            (RelationshipAuthority::VerifiedSameSnapshotCompilationDependency, None) => {
                return Err(invalid(
                    "verified compilation dependency has no sealed evidence reference",
                ));
            }
            (RelationshipAuthority::DeclaredTopology | RelationshipAuthority::Unbound, None) => {}
            _ => {
                return Err(invalid(
                    "declared or unbound topology cannot carry verified dependency evidence",
                ));
            }
        }
    }
    request.pairs.sort_by(|left, right| {
        (
            left.pair_id.as_str(),
            left.provider_member.as_str(),
            left.consumer_member.as_str(),
        )
            .cmp(&(
                right.pair_id.as_str(),
                right.provider_member.as_str(),
                right.consumer_member.as_str(),
            ))
    });
    if request
        .pairs
        .windows(2)
        .any(|pair| pair[0].pair_id == pair[1].pair_id)
    {
        return Err(invalid("callable pair ids must be unique"));
    }
    let pairs = request
        .pairs
        .iter()
        .map(|pair| pair.pair_id.as_str())
        .collect::<BTreeSet<_>>();
    let mut all_terms = BTreeSet::new();
    for task in &mut request.tasks {
        validate_alias(&task.task_id, "task id")?;
        validate_alias(&task.pair_id, "task pair id")?;
        if !pairs.contains(task.pair_id.as_str()) || task.terms.is_empty() {
            return Err(invalid(
                "callable task references an unknown pair or has no terms",
            ));
        }
        let mut normalized = BTreeSet::new();
        for term in &task.terms {
            validate_text(term, "query term")?;
            if ["callable:", "constructor:", "property:", "class:"]
                .iter()
                .any(|prefix| term.starts_with(prefix))
            {
                crate::semantic_validation::validate_kotlin_full_symbol_identity(term)
                    .map_err(|_| invalid("tagged callable task term is malformed or unsafe"))?;
                normalized.insert(term.clone());
            } else {
                normalized.extend(identifier_terms(term)?);
            }
        }
        if normalized.is_empty() {
            return Err(invalid("callable task has no normalized identifier terms"));
        }
        task.terms = normalized.into_iter().collect();
        all_terms.extend(task.terms.iter().cloned());
    }
    request.tasks.sort_by(|left, right| {
        (left.task_id.as_str(), left.pair_id.as_str())
            .cmp(&(right.task_id.as_str(), right.pair_id.as_str()))
    });
    if request
        .tasks
        .windows(2)
        .any(|pair| pair[0].task_id == pair[1].task_id)
        || all_terms.len() > request.budgets.max_query_terms
    {
        return Err(invalid(
            "callable task ids repeat or normalized terms exceed the thread-global bound",
        ));
    }
    Ok(())
}

fn validate_qualified_payload(
    input: &QualifiedCallablePayload,
    budgets: &CallableBudgets,
) -> Result<usize, ClewError> {
    validate_member_authority(&input.member)?;
    validate_compilation_authority(&input.compilation)?;
    validate_text(&input.fact_key, "generation fact key")?;
    validate_cas_object(&input.payload_ref)?;
    if input.payload_ref.object_schema != KOTLIN_SEMANTIC_FACT_SCHEMA {
        return Err(invalid("callable payload has the wrong CAS object schema"));
    }
    if let Some(reference) = &input.source_ref {
        validate_cas_object(reference)?;
    }
    validate_json_strings(&input.payload, budgets.max_text_bytes, 0)?;
    let schema = payload_schema(input)
        .ok_or_else(|| invalid("qualified Kotlin payload has no supported closed schema"))?;
    if !matches!(
        schema,
        "declaration-descriptor/0.1"
            | "declaration-relation/0.1"
            | "declaration-descriptor-boundary/0.1"
            | "declaration-relation-boundary/0.1"
    ) {
        return Err(invalid("qualified Kotlin payload schema is outside S1K"));
    }
    crate::semantic_validation::validate_kotlin_semantic_payload(&input.payload)
        .map_err(|_| invalid("qualified Kotlin payload fails its closed semantic validator"))?;
    validate_payload_authority(input, schema)?;
    let bytes = canonical::bytes(&input.payload).map_err(internal)?;
    let expected = CasObject::for_bytes(KOTLIN_SEMANTIC_FACT_SCHEMA, &bytes)?;
    if expected != input.payload_ref {
        return Err(invalid(
            "qualified Kotlin payload differs from its CAS authority",
        ));
    }
    let category = payload_category(schema);
    let hash = canonical::hash_bytes(&bytes);
    let expected_key = format!(
        "kotlin:{category}:{}",
        hash.strip_prefix("sha256:").unwrap_or(&hash)
    );
    if input.fact_key != expected_key {
        return Err(invalid("qualified Kotlin payload fact key is inconsistent"));
    }
    let has_file = input.payload.get("file").is_some();
    if has_file != input.source_ref.is_some() {
        return Err(invalid(
            "qualified Kotlin payload source path and source CAS authority disagree",
        ));
    }
    if let Some(path) = input.payload.get("file").and_then(Value::as_str) {
        validate_relative_path(path)?;
    }
    Ok(bytes.len())
}

fn validate_selected_compilation(selected: &CallableSelectedCompilation) -> Result<(), ClewError> {
    validate_member_authority(&selected.member)?;
    validate_compilation_authority(&selected.compilation)
}

fn validate_member_authority(member: &CallableMemberAuthority) -> Result<(), ClewError> {
    validate_alias(&member.member_alias, "member alias")?;
    validate_alias(&member.service_alias, "service alias")?;
    validate_text(&member.session_id, "session id")?;
    validate_digest(&member.session_authority_digest, "session authority digest")?;
    repository_namespace(&member.repository_key)?;
    validate_text(&member.base_revision, "base revision")?;
    validate_cas_object(&member.snapshot_ref)
}

fn validate_compilation_authority(
    compilation: &CallableCompilationAuthority,
) -> Result<(), ClewError> {
    validate_text(&compilation.compilation_id, "compilation id")?;
    validate_digest(&compilation.generation_id, "generation id")?;
    validate_cas_object(&compilation.generation_ref)?;
    if compilation.semantic_authority != "K2_FIR"
        || compilation.extractor_id != "fir-facts-extractor/0.6"
    {
        return Err(invalid(
            "callable input is not qualified K2 FIR descriptor evidence",
        ));
    }
    validate_digest(&compilation.adapter_digest, "adapter digest")?;
    validate_digest(&compilation.runtime_digest, "runtime digest")
}

fn validate_payload_authority(
    input: &QualifiedCallablePayload,
    schema: &str,
) -> Result<(), ClewError> {
    let payload = &input.payload;
    match schema {
        "declaration-descriptor/0.1" => {
            if payload.get("resolution").and_then(Value::as_str) != Some("PROVEN")
                || payload.get("provider").and_then(Value::as_str) != Some("K2_FIR")
                || payload.get("compilerAuthority").and_then(Value::as_str)
                    != Some("fir-facts-extractor/0.6")
                || payload.get("sourceProvenance").and_then(Value::as_str)
                    != Some("COMPILER_UTF16_RANGE_TO_UTF8_BYTES")
            {
                return Err(invalid("declaration payload is not qualified K2 evidence"));
            }
            let partial =
                payload.get("attributeCoverage").and_then(Value::as_str) == Some("PARTIAL");
            if partial && input.compilation.descriptor_coverage != GraphCoverage::Partial {
                return Err(invalid(
                    "partial declaration row has complete compilation coverage",
                ));
            }
        }
        "declaration-relation/0.1" => {
            if payload.get("resolution").and_then(Value::as_str) != Some("PROVEN")
                || payload.get("provider").and_then(Value::as_str) != Some("K2_FIR")
                || payload.get("sourceProvenance").and_then(Value::as_str)
                    != Some("COMPILER_UTF16_RANGE_TO_UTF8_BYTES")
            {
                return Err(invalid("relation payload is not qualified K2 evidence"));
            }
            let partial =
                payload.get("attributeCoverage").and_then(Value::as_str) == Some("PARTIAL");
            if partial && input.compilation.relation_coverage != GraphCoverage::Partial {
                return Err(invalid(
                    "partial relation row has complete compilation coverage",
                ));
            }
        }
        "declaration-descriptor-boundary/0.1" => {
            if input.compilation.descriptor_coverage != GraphCoverage::Partial
                || payload.get("resolution").and_then(Value::as_str) != Some("UNKNOWN")
            {
                return Err(invalid(
                    "descriptor boundary lacks partial/unknown authority",
                ));
            }
        }
        "declaration-relation-boundary/0.1" => {
            if input.compilation.relation_coverage != GraphCoverage::Partial
                || payload.get("resolution").and_then(Value::as_str) != Some("UNKNOWN")
            {
                return Err(invalid("relation boundary lacks partial/unknown authority"));
            }
        }
        _ => unreachable!(),
    }
    Ok(())
}

fn member_bindings(
    selected_compilations: &[CallableSelectedCompilation],
    budgets: &CallableBudgets,
) -> Result<Vec<CallableMemberBinding>, ClewError> {
    let mut authorities = BTreeMap::<String, CallableMemberAuthority>::new();
    let mut compilations = BTreeMap::<(String, String), CallableCompilationAuthority>::new();
    for selected in selected_compilations {
        match authorities.get(&selected.member.member_alias) {
            Some(existing) if existing != &selected.member => {
                return Err(invalid(
                    "member alias substitution changed exact session/repository authority",
                ));
            }
            None => {
                authorities.insert(
                    selected.member.member_alias.clone(),
                    selected.member.clone(),
                );
            }
            _ => {}
        }
        let key = (
            selected.member.member_alias.clone(),
            selected.compilation.compilation_id.clone(),
        );
        match compilations.get(&key) {
            Some(existing) if existing != &selected.compilation => {
                return Err(invalid(
                    "compilation identity changed generation/runtime authority",
                ));
            }
            None => {
                compilations.insert(key, selected.compilation.clone());
            }
            _ => {}
        }
    }
    if authorities.len() < 2 || authorities.len() > budgets.max_members {
        return Err(budget(
            "callable fact set must bind between two and eight members",
        ));
    }
    if compilations.len() > budgets.max_compilations {
        return Err(budget("callable selected compilations exceed 64"));
    }
    let mut sessions = BTreeSet::new();
    let mut namespaces = BTreeSet::new();
    let mut members = Vec::with_capacity(authorities.len());
    for (alias, member) in authorities {
        let namespace = repository_namespace(&member.repository_key)?;
        if !sessions.insert(member.session_id.clone()) || !namespaces.insert(namespace.clone()) {
            return Err(invalid(
                "callable members repeat a session or repository namespace",
            ));
        }
        let selected = compilations
            .iter()
            .filter(|((member_alias, _), _)| member_alias == &alias)
            .map(|(_, authority)| CallableCompilationBinding {
                compilation_id: authority.compilation_id.clone(),
                generation_id: authority.generation_id.clone(),
                generation_ref: authority.generation_ref.clone(),
                semantic_authority: authority.semantic_authority.clone(),
                extractor_id: authority.extractor_id.clone(),
                adapter_digest: authority.adapter_digest.clone(),
                runtime_digest: authority.runtime_digest.clone(),
                descriptor_coverage: authority.descriptor_coverage,
                relation_coverage: authority.relation_coverage,
            })
            .collect::<Vec<_>>();
        members.push(CallableMemberBinding {
            member_alias: alias,
            service_alias: member.service_alias,
            session_id: member.session_id,
            session_authority_digest: member.session_authority_digest,
            repository_key: member.repository_key,
            repository_namespace: namespace,
            base_revision: member.base_revision,
            snapshot_ref: member.snapshot_ref,
            compilations: selected,
        });
    }
    Ok(members)
}

fn validate_pair_members(
    request: &CallableFactSetRequest,
    members: &[CallableMemberBinding],
) -> Result<(), ClewError> {
    let known = members
        .iter()
        .map(|member| member.member_alias.as_str())
        .collect::<BTreeSet<_>>();
    for pair in &request.pairs {
        if !known.contains(pair.provider_member.as_str())
            || !known.contains(pair.consumer_member.as_str())
        {
            return Err(invalid("callable pair references an unknown Kotlin member"));
        }
    }
    if members.iter().any(|member| {
        !request.pairs.iter().any(|pair| {
            pair.provider_member == member.member_alias
                || pair.consumer_member == member.member_alias
        })
    }) {
        return Err(invalid("callable member is not bound by any declared pair"));
    }
    Ok(())
}

fn normalize_declaration(
    input: &QualifiedCallablePayload,
    boundaries: &[BoundaryFact],
    budgets: &CallableBudgets,
) -> Result<DeclarationFact, ClewError> {
    let payload = &input.payload;
    let kind = match required_string(payload, "declarationKind")? {
        "FUNCTION" => DeclarationKind::Function,
        "CONSTRUCTOR" => DeclarationKind::Constructor,
        "CLASS" => DeclarationKind::Class,
        "PROPERTY" => DeclarationKind::Property,
        "MUTABLE_PROPERTY" => DeclarationKind::MutableProperty,
        _ => return Err(invalid("declaration has an unsupported kind")),
    };
    let symbol_identity = required_bounded_string(payload, "symbolIdentity", budgets)?;
    let owner_identity = required_bounded_string(payload, "ownerIdentity", budgets)?;
    let containment = string_array(payload, "containment", budgets.max_containment_depth)?;
    if containment.last().map(String::as_str).map_or_else(
        || !owner_identity.starts_with("package:"),
        |owner| owner != owner_identity,
    ) {
        return Err(invalid(
            "declaration containment disagrees with its package/class owner",
        ));
    }
    let compiler_callable_id = optional_bounded_string(payload, "compilerCallableId", budgets)?;
    let compiler_class_id = optional_bounded_string(payload, "compilerClassId", budgets)?;
    let jvm_descriptor =
        optional_bounded_string(payload, "jvmDescriptor", budgets)?.or_else(|| {
            matches!(kind, DeclarationKind::Function)
                .then(|| {
                    symbol_identity
                        .split_once("#jvm:")
                        .map(|(_, value)| value.to_owned())
                })
                .flatten()
        });
    validate_declaration_identity(
        kind,
        &symbol_identity,
        compiler_callable_id.as_deref(),
        compiler_class_id.as_deref(),
        jvm_descriptor.as_deref(),
    )?;
    validate_declaration_limits(payload, budgets)?;
    let partial_row = payload.get("attributeCoverage").and_then(Value::as_str) == Some("PARTIAL")
        || payload.get("sourceRowHash").is_some();
    let mut uncertainty_reasons = BTreeSet::new();
    if partial_row {
        uncertainty_reasons.insert("PARTIAL_DECLARATION".to_owned());
    }
    uncertainty_reasons.extend(descriptor_boundary_reasons(
        input,
        &symbol_identity,
        compiler_callable_id.as_deref(),
        compiler_class_id.as_deref(),
        &owner_identity,
        &containment,
        boundaries,
    ));
    let exact_eligible = uncertainty_reasons.is_empty();
    let projected_shape = projected_payload(payload);
    let shape_digest = exact_eligible
        .then(|| canonical::hash(&projected_shape).map_err(internal))
        .transpose()?;
    let mut row = DeclarationFact {
        schema: CALLABLE_FACT_SCHEMA.into(),
        fact_id: String::new(),
        provenance: provenance(input)?,
        declaration_kind: kind,
        symbol_identity,
        compiler_callable_id,
        compiler_class_id,
        jvm_descriptor,
        owner_identity,
        containment,
        projected_shape,
        shape_digest,
        exact_eligible,
        uncertainty_reasons: uncertainty_reasons.into_iter().collect(),
    };
    row.fact_id = declaration_fact_id(&row)?;
    Ok(row)
}

#[allow(clippy::too_many_arguments)]
fn descriptor_boundary_reasons(
    input: &QualifiedCallablePayload,
    symbol_identity: &str,
    compiler_callable_id: Option<&str>,
    compiler_class_id: Option<&str>,
    owner_identity: &str,
    containment: &[String],
    boundaries: &[BoundaryFact],
) -> BTreeSet<String> {
    boundaries
        .iter()
        .filter(|boundary| {
            boundary
                .provenance
                .input_fact_key
                .starts_with("kotlin:descriptor-boundary:")
                && boundary.provenance.member_alias == input.member.member_alias
                && boundary.provenance.compilation_id == input.compilation.compilation_id
        })
        .filter(|boundary| {
            boundary.subject.as_deref().is_some_and(|subject| {
                subject == symbol_identity
                    || compiler_callable_id == Some(subject)
                    || compiler_class_id == Some(subject)
                    || subject == owner_identity
                    || containment.iter().any(|owner| owner == subject)
            })
        })
        .map(|boundary| format!("BOUNDARY_{}", boundary.code))
        .collect()
}

fn normalize_use(
    input: &QualifiedCallablePayload,
    request: &CallableFactSetRequest,
    declarations: &BTreeMap<(String, String), &DeclarationFact>,
    member_namespaces: &BTreeMap<String, String>,
    boundaries: &[BoundaryFact],
    budgets: &CallableBudgets,
) -> Result<UseFact, ClewError> {
    let payload = &input.payload;
    let relation_kind = required_bounded_string(payload, "kind", budgets)?;
    let source_owner = required_bounded_string(payload, "owner", budgets)?;
    let raw_target = required_bounded_string(payload, "target", budgets)?;
    let partial_row = payload.get("attributeCoverage").and_then(Value::as_str) == Some("PARTIAL")
        || payload.get("sourceRowHash").is_some();
    let mut uncertainty_reasons = BTreeSet::new();
    if partial_row {
        uncertainty_reasons.insert("PARTIAL_USE".to_owned());
    }
    let target_is_full_symbol = full_symbol_identity(&raw_target);
    uncertainty_reasons.extend(relation_boundary_reasons(
        input,
        &source_owner,
        &raw_target,
        boundaries,
    ));
    let source_namespace = member_namespaces
        .get(&input.member.member_alias)
        .ok_or_else(|| invalid("use source member has no repository namespace"))?;
    let mut candidates = BTreeSet::<(String, String, RelationshipAuthority)>::new();
    if target_is_full_symbol {
        if let Some(declaration) = declarations.get(&(source_namespace.clone(), raw_target.clone()))
            && declaration.exact_eligible
            && declaration.provenance.compilation_id == input.compilation.compilation_id
        {
            candidates.insert((
                source_namespace.clone(),
                declaration.symbol_identity.clone(),
                RelationshipAuthority::VerifiedSameSnapshotCompilationDependency,
            ));
        }
        for pair in request.pairs.iter().filter(|pair| {
            pair.consumer_member == input.member.member_alias
                && pair.relationship_authority
                    == RelationshipAuthority::VerifiedSameSnapshotCompilationDependency
        }) {
            let Some(namespace) = member_namespaces.get(&pair.provider_member) else {
                continue;
            };
            if declarations
                .get(&(namespace.clone(), raw_target.clone()))
                .is_some_and(|declaration| declaration.exact_eligible)
            {
                candidates.insert((
                    namespace.clone(),
                    raw_target.clone(),
                    RelationshipAuthority::VerifiedSameSnapshotCompilationDependency,
                ));
            }
        }
    } else {
        for declaration in declarations.values().filter(|declaration| {
            declaration.exact_eligible
                && declaration.provenance.repository_namespace == *source_namespace
                && declaration.provenance.compilation_id == input.compilation.compilation_id
                && declaration.compiler_callable_id.as_deref() == Some(raw_target.as_str())
        }) {
            candidates.insert((
                source_namespace.clone(),
                declaration.symbol_identity.clone(),
                RelationshipAuthority::VerifiedSameSnapshotCompilationDependency,
            ));
        }
        for pair in request.pairs.iter().filter(|pair| {
            pair.consumer_member == input.member.member_alias
                && pair.relationship_authority
                    == RelationshipAuthority::VerifiedSameSnapshotCompilationDependency
        }) {
            let Some(namespace) = member_namespaces.get(&pair.provider_member) else {
                continue;
            };
            for declaration in declarations.values().filter(|declaration| {
                declaration.exact_eligible
                    && declaration.provenance.repository_namespace == *namespace
                    && declaration.compiler_callable_id.as_deref() == Some(raw_target.as_str())
            }) {
                candidates.insert((
                    namespace.clone(),
                    declaration.symbol_identity.clone(),
                    RelationshipAuthority::VerifiedSameSnapshotCompilationDependency,
                ));
            }
        }
    }
    let exact_target = if uncertainty_reasons.is_empty() && candidates.len() == 1 {
        candidates.iter().next().cloned()
    } else {
        None
    };
    if target_is_full_symbol && candidates.len() > 1 {
        uncertainty_reasons.insert("AMBIGUOUS_FULL_SYMBOL_TARGET".to_owned());
    } else if target_is_full_symbol && exact_target.is_none() {
        uncertainty_reasons.insert("UNBOUND_FULL_SYMBOL_TARGET".to_owned());
    }
    let declared_relationship = request.pairs.iter().any(|pair| {
        pair.consumer_member == input.member.member_alias
            && pair.relationship_authority == RelationshipAuthority::DeclaredTopology
    });
    let (target_resolution, target_symbol_identity, target_namespace, relationship_authority) =
        match exact_target {
            Some((namespace, symbol_identity, authority)) => (
                TargetResolution::ExactSymbol,
                Some(symbol_identity),
                Some(namespace),
                authority,
            ),
            None => (
                TargetResolution::CallableFamily,
                None,
                None,
                if declared_relationship {
                    RelationshipAuthority::DeclaredTopology
                } else {
                    RelationshipAuthority::Unbound
                },
            ),
        };
    let target_callable_id = if target_resolution == TargetResolution::ExactSymbol {
        declarations
            .get(&(
                target_namespace.clone().expect("exact target namespace"),
                target_symbol_identity
                    .clone()
                    .expect("exact target symbol identity"),
            ))
            .and_then(|declaration| declaration.compiler_callable_id.clone())
            .or_else(|| callable_family_from_symbol(&raw_target))
            .unwrap_or_else(|| raw_target.clone())
    } else {
        callable_family_from_symbol(&raw_target).unwrap_or_else(|| raw_target.clone())
    };
    let exact_eligible = target_resolution == TargetResolution::ExactSymbol
        && relationship_authority
            == RelationshipAuthority::VerifiedSameSnapshotCompilationDependency
        && uncertainty_reasons.is_empty();
    let mut row = UseFact {
        schema: CALLABLE_FACT_SCHEMA.into(),
        fact_id: String::new(),
        provenance: provenance(input)?,
        relation_kind,
        source_owner,
        target_callable_id,
        target_symbol_identity,
        target_repository_namespace: target_namespace,
        target_resolution,
        relationship_authority,
        relation_evidence: projected_payload(payload),
        exact_eligible,
        uncertainty_reasons: uncertainty_reasons.into_iter().collect(),
    };
    row.fact_id = use_fact_id(&row)?;
    Ok(row)
}

fn relation_boundary_reasons(
    input: &QualifiedCallablePayload,
    source_owner: &str,
    raw_target: &str,
    boundaries: &[BoundaryFact],
) -> BTreeSet<String> {
    let same_compilation = boundaries
        .iter()
        .filter(|boundary| {
            boundary.provenance.member_alias == input.member.member_alias
                && boundary.provenance.compilation_id == input.compilation.compilation_id
                && boundary
                    .provenance
                    .input_fact_key
                    .starts_with("kotlin:relation-boundary:")
        })
        .collect::<Vec<_>>();
    let matching = same_compilation
        .iter()
        .filter(|boundary| relation_boundary_affects_target_resolution(boundary))
        .filter(|boundary| {
            boundary.subject.as_deref().is_none_or(|subject| {
                subject == source_owner
                    || subject == raw_target
                    || callable_family_from_symbol(raw_target).as_deref() == Some(subject)
            })
        })
        .map(|boundary| format!("BOUNDARY_{}", boundary.code))
        .collect::<BTreeSet<_>>();
    if matching.is_empty()
        && input.compilation.relation_coverage == GraphCoverage::Partial
        && same_compilation.is_empty()
    {
        BTreeSet::from(["PARTIAL_RELATION_COVERAGE".into()])
    } else {
        matching
    }
}

fn relation_boundary_affects_target_resolution(boundary: &BoundaryFact) -> bool {
    matches!(
        boundary.stage.as_str(),
        "CALL_RESOLUTION" | "CONSTRUCTOR_RESOLUTION" | "TARGET_RESOLUTION" | "RELATION_RESOLUTION"
    )
}

fn normalize_boundary(
    input: &QualifiedCallablePayload,
    budgets: &CallableBudgets,
) -> Result<BoundaryFact, ClewError> {
    let payload = &input.payload;
    let stage = required_bounded_string(payload, "stage", budgets)?;
    let code = required_bounded_string(payload, "code", budgets)?;
    let subject = [
        "subject",
        "symbolIdentity",
        "owner",
        "target",
        "compilerCallableId",
        "compilerClassId",
    ]
    .into_iter()
    .find_map(|field| {
        payload
            .get(field)
            .and_then(Value::as_str)
            .map(str::to_owned)
    });
    if let Some(subject) = &subject {
        validate_text(subject, "boundary subject")?;
    }
    let check = format!("VERIFY_{code}");
    validate_text(&check, "boundary required check")?;
    let mut row = BoundaryFact {
        schema: CALLABLE_FACT_SCHEMA.into(),
        fact_id: String::new(),
        provenance: provenance(input)?,
        stage,
        code,
        subject,
        required_checks: vec![check],
        boundary_evidence: projected_payload(payload),
    };
    row.fact_id = boundary_fact_id(&row)?;
    Ok(row)
}

fn provenance(input: &QualifiedCallablePayload) -> Result<CallableFactProvenance, ClewError> {
    Ok(CallableFactProvenance {
        member_alias: input.member.member_alias.clone(),
        repository_namespace: repository_namespace(&input.member.repository_key)?,
        session_id: input.member.session_id.clone(),
        session_authority_digest: input.member.session_authority_digest.clone(),
        base_revision: input.member.base_revision.clone(),
        compilation_id: input.compilation.compilation_id.clone(),
        generation_id: input.compilation.generation_id.clone(),
        generation_ref: input.compilation.generation_ref.clone(),
        input_fact_key: input.fact_key.clone(),
        input_payload_ref: input.payload_ref.clone(),
        source: source_anchor(input)?,
    })
}

fn source_anchor(input: &QualifiedCallablePayload) -> Result<Option<SourceAnchor>, ClewError> {
    let Some(path) = input.payload.get("file").and_then(Value::as_str) else {
        return Ok(None);
    };
    let start = input.payload.get("start").and_then(Value::as_u64);
    let end = input.payload.get("end").and_then(Value::as_u64);
    if start.is_some() != end.is_some() || start.zip(end).is_some_and(|(start, end)| end < start) {
        return Err(invalid("callable source range is incomplete or reversed"));
    }
    Ok(Some(SourceAnchor {
        path: path.to_owned(),
        start,
        end,
        content_ref: input
            .source_ref
            .clone()
            .ok_or_else(|| invalid("callable source anchor has no content authority"))?,
    }))
}

fn projected_payload(payload: &Value) -> Value {
    let omitted = BTreeSet::from([
        "schema",
        "file",
        "start",
        "end",
        "resolution",
        "provider",
        "module",
        "sourceSet",
        "sourceProvenance",
        "compilerAuthority",
        "attributeCoverage",
        "sourceRowHash",
    ]);
    let entries = payload
        .as_object()
        .into_iter()
        .flatten()
        .filter(|(key, _)| !omitted.contains(key.as_str()))
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect::<Map<_, _>>();
    Value::Object(entries)
}

fn validate_declaration_identity(
    kind: DeclarationKind,
    identity: &str,
    callable: Option<&str>,
    class: Option<&str>,
    jvm: Option<&str>,
) -> Result<(), ClewError> {
    let valid = match kind {
        DeclarationKind::Function => matches!((callable, jvm), (Some(callable), Some(jvm))
            if identity == format!("callable:{callable}#jvm:{jvm}")),
        DeclarationKind::Constructor => {
            matches!((callable, class, jvm), (Some(callable), Some(_), Some(jvm))
                if identity == format!("constructor:{callable}#jvm:{jvm}"))
        }
        DeclarationKind::Property | DeclarationKind::MutableProperty => {
            callable.is_some_and(|callable| identity == format!("property:{callable}"))
        }
        DeclarationKind::Class => class.is_some_and(|class| identity == format!("class:{class}")),
    };
    if !valid {
        return Err(invalid(
            "declaration full symbol identity disagrees with compiler/JVM identity",
        ));
    }
    Ok(())
}

fn validate_declaration_limits(
    payload: &Value,
    budgets: &CallableBudgets,
) -> Result<(), ClewError> {
    if payload
        .get("parameterTypes")
        .and_then(Value::as_array)
        .is_some_and(|values| values.len() > budgets.max_parameters_per_callable)
    {
        return Err(budget("callable parameter count exceeds 1,024"));
    }
    let type_parameters = payload
        .get("typeParameters")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    if type_parameters.len() > budgets.max_type_parameters {
        return Err(budget("declaration type parameter count exceeds 256"));
    }
    for parameter in type_parameters {
        if parameter
            .get("bounds")
            .and_then(Value::as_array)
            .is_some_and(|bounds| bounds.len() > budgets.max_bounds_per_type_parameter)
        {
            return Err(budget("type parameter bound count exceeds 64"));
        }
    }
    Ok(())
}

fn reject_duplicate_declarations(rows: &[DeclarationFact]) -> Result<(), ClewError> {
    let mut identities = BTreeSet::new();
    for row in rows {
        if !identities.insert((
            row.provenance.repository_namespace.as_str(),
            row.symbol_identity.as_str(),
        )) {
            return Err(invalid(
                "repository namespace repeats a declaration symbol identity",
            ));
        }
    }
    Ok(())
}

fn declaration_fact_id(row: &DeclarationFact) -> Result<String, ClewError> {
    let mut unsigned = row.clone();
    unsigned.fact_id.clear();
    canonical::hash(&CallableFact::Declaration(unsigned)).map_err(internal)
}

fn use_fact_id(row: &UseFact) -> Result<String, ClewError> {
    let mut unsigned = row.clone();
    unsigned.fact_id.clear();
    canonical::hash(&CallableFact::Use(unsigned)).map_err(internal)
}

fn boundary_fact_id(row: &BoundaryFact) -> Result<String, ClewError> {
    let mut unsigned = row.clone();
    unsigned.fact_id.clear();
    canonical::hash(&CallableFact::Boundary(unsigned)).map_err(internal)
}

fn validate_fact_counts(
    budgets: &CallableBudgets,
    declarations: usize,
    uses: usize,
    boundaries: usize,
) -> Result<(), ClewError> {
    let total = declarations
        .checked_add(uses)
        .and_then(|value| value.checked_add(boundaries))
        .ok_or_else(|| budget("normalized callable fact count overflowed"))?;
    if declarations == 0 {
        return Err(invalid("callable fact set has no Kotlin declarations"));
    }
    if declarations > budgets.max_declarations
        || uses > budgets.max_uses
        || boundaries > budgets.max_boundaries
        || total > budgets.max_normalized_facts
    {
        return Err(budget(
            "normalized callable fact counts exceed frozen bounds",
        ));
    }
    Ok(())
}

fn build_fact_shards(
    facts: &[CallableFact],
    budgets: &CallableBudgets,
) -> Result<(Vec<PreparedCasObject>, Vec<CallableFactShardReference>), ClewError> {
    let mut prepared = Vec::new();
    let mut references = Vec::new();
    let encoded_lengths = facts
        .iter()
        .map(|fact| {
            canonical::bytes(fact)
                .map(|bytes| bytes.len())
                .map_err(internal)
        })
        .collect::<Result<Vec<_>, _>>()?;
    let mut start = 0usize;
    let mut encoded_rows = 0usize;
    let mut sequence = 0u32;
    for end in 0..facts.len() {
        let candidate_rows = encoded_rows
            .checked_add(encoded_lengths[end])
            .ok_or_else(|| budget("callable fact shard size overflowed"))?;
        let candidate_count = end - start + 1;
        if encoded_fact_shard_len(
            sequence,
            facts[start].fact_id(),
            facts[end].fact_id(),
            candidate_rows,
            candidate_count,
        )? <= budgets.max_shard_bytes
        {
            encoded_rows = candidate_rows;
            continue;
        }
        if end == start {
            return Err(budget(
                "one normalized callable fact exceeds the 8 MiB shard bound",
            ));
        }
        publish_prepared_fact_shard(
            sequence,
            &facts[start..end],
            &mut prepared,
            &mut references,
            budgets,
        )?;
        sequence = sequence
            .checked_add(1)
            .ok_or_else(|| budget("callable fact shard sequence overflowed"))?;
        start = end;
        encoded_rows = encoded_lengths[end];
        if encoded_fact_shard_len(
            sequence,
            facts[end].fact_id(),
            facts[end].fact_id(),
            encoded_rows,
            1,
        )? > budgets.max_shard_bytes
        {
            return Err(budget(
                "one normalized callable fact exceeds the 8 MiB shard bound",
            ));
        }
    }
    if start < facts.len() {
        publish_prepared_fact_shard(
            sequence,
            &facts[start..],
            &mut prepared,
            &mut references,
            budgets,
        )?;
    }
    Ok((prepared, references))
}

fn encoded_fact_shard_len(
    sequence: u32,
    first_fact_id: &str,
    last_fact_id: &str,
    encoded_rows: usize,
    row_count: usize,
) -> Result<usize, ClewError> {
    encoded_closed_object_len(&[
        ("facts", encoded_array_len(encoded_rows, row_count)?),
        ("firstFactId", encoded_ascii_string_len(first_fact_id)?),
        ("lastFactId", encoded_ascii_string_len(last_fact_id)?),
        (
            "schema",
            encoded_ascii_string_len(CALLABLE_FACT_SHARD_SCHEMA)?,
        ),
        ("sequence", sequence.to_string().len()),
    ])
}

fn callable_fact_shard(
    sequence: u32,
    facts: &[CallableFact],
) -> Result<CallableFactShard, ClewError> {
    let first = facts
        .first()
        .ok_or_else(|| invalid("callable fact shard is empty"))?;
    let last = facts.last().expect("non-empty checked above");
    Ok(CallableFactShard {
        schema: CALLABLE_FACT_SHARD_SCHEMA.into(),
        sequence,
        first_fact_id: first.fact_id().to_owned(),
        last_fact_id: last.fact_id().to_owned(),
        facts: facts.to_vec(),
    })
}

fn publish_prepared_fact_shard(
    sequence: u32,
    facts: &[CallableFact],
    prepared: &mut Vec<PreparedCasObject>,
    references: &mut Vec<CallableFactShardReference>,
    budgets: &CallableBudgets,
) -> Result<(), ClewError> {
    let shard = callable_fact_shard(sequence, facts)?;
    #[cfg(test)]
    FACT_SHARD_SERIALIZATIONS.with(|count| count.set(count.get() + 1));
    let bytes = canonical::bytes(&shard).map_err(internal)?;
    if bytes.len() > budgets.max_shard_bytes {
        return Err(budget("callable fact shard exceeds 8 MiB"));
    }
    let object = CasObject::for_bytes(CALLABLE_FACT_SHARD_SCHEMA, &bytes)?;
    references.push(CallableFactShardReference {
        sequence,
        first_fact_id: shard.first_fact_id.clone(),
        last_fact_id: shard.last_fact_id.clone(),
        object: object.clone(),
    });
    prepared.push(PreparedCasObject {
        reference: object,
        bytes,
    });
    Ok(())
}

fn build_index_records(facts: &[CallableFact]) -> Result<Vec<CallableIndexRecord>, ClewError> {
    let mut records = BTreeSet::new();
    for fact in facts {
        let posting = posting_for_fact(fact);
        let mut keys = BTreeSet::new();
        let mut searchable = BTreeSet::new();
        match fact {
            CallableFact::Declaration(row) => {
                keys.insert(CallableIndexKey {
                    kind: CallableLookupKind::FullSymbol,
                    repository_namespace: Some(row.provenance.repository_namespace.clone()),
                    value: row.symbol_identity.clone(),
                });
                if let Some(callable) = row
                    .compiler_callable_id
                    .as_ref()
                    .or(row.compiler_class_id.as_ref())
                {
                    keys.insert(CallableIndexKey {
                        kind: CallableLookupKind::CallableFamily,
                        repository_namespace: Some(row.provenance.repository_namespace.clone()),
                        value: callable.clone(),
                    });
                    keys.insert(CallableIndexKey {
                        kind: CallableLookupKind::CallableFamily,
                        repository_namespace: None,
                        value: callable.clone(),
                    });
                    searchable.insert(callable.clone());
                }
                searchable.extend([row.symbol_identity.clone(), row.owner_identity.clone()]);
                searchable.extend(row.containment.iter().cloned());
                collect_json_index_strings(&row.projected_shape, &mut searchable, 0)?;
            }
            CallableFact::Use(row) => {
                if row.target_resolution == TargetResolution::ExactSymbol {
                    keys.insert(CallableIndexKey {
                        kind: CallableLookupKind::FullSymbol,
                        repository_namespace: row.target_repository_namespace.clone(),
                        value: row
                            .target_symbol_identity
                            .clone()
                            .expect("exact target always has a full identity"),
                    });
                }
                keys.insert(CallableIndexKey {
                    kind: CallableLookupKind::CallableFamily,
                    repository_namespace: row.target_repository_namespace.clone(),
                    value: row.target_callable_id.clone(),
                });
                if row.target_repository_namespace.is_some() {
                    keys.insert(CallableIndexKey {
                        kind: CallableLookupKind::CallableFamily,
                        repository_namespace: None,
                        value: row.target_callable_id.clone(),
                    });
                }
                searchable.extend([
                    row.source_owner.clone(),
                    row.target_callable_id.clone(),
                    row.relation_kind.clone(),
                ]);
                if let Some(symbol) = &row.target_symbol_identity {
                    searchable.insert(symbol.clone());
                }
                collect_json_index_strings(&row.relation_evidence, &mut searchable, 0)?;
            }
            CallableFact::Boundary(row) => {
                searchable.extend([row.stage.clone(), row.code.clone()]);
                if let Some(subject) = &row.subject {
                    searchable.insert(subject.clone());
                }
            }
        }
        let mut terms = BTreeSet::new();
        for value in &searchable {
            if !is_digest(value) {
                terms.extend(identifier_terms(value)?);
            }
            if terms.len() > MAX_INDEX_TERMS_PER_FACT {
                return Err(budget("one callable fact produces too many index terms"));
            }
        }
        keys.extend(terms.into_iter().map(|term| CallableIndexKey {
            kind: CallableLookupKind::Token,
            repository_namespace: None,
            value: term,
        }));
        for key in keys {
            records.insert(CallableIndexRecord {
                key,
                posting: posting.clone(),
            });
        }
    }
    Ok(records.into_iter().collect())
}

fn posting_for_fact(fact: &CallableFact) -> CallablePosting {
    let provenance = fact.provenance();
    let source = provenance.source.as_ref();
    let base = |fact_kind, symbol_identity, callable_id, exact_eligible, uncertainty_reasons| {
        CallablePosting {
            fact_id: fact.fact_id().to_owned(),
            fact_kind,
            member_alias: provenance.member_alias.clone(),
            repository_namespace: provenance.repository_namespace.clone(),
            symbol_identity,
            callable_id,
            source_path: source.map(|source| source.path.clone()),
            source_start: source.and_then(|source| source.start),
            source_end: source.and_then(|source| source.end),
            exact_eligible,
            uncertainty_reasons,
        }
    };
    match fact {
        CallableFact::Declaration(row) => base(
            CallableFactKind::Declaration,
            Some(row.symbol_identity.clone()),
            row.compiler_callable_id
                .clone()
                .or_else(|| row.compiler_class_id.clone()),
            row.exact_eligible,
            row.uncertainty_reasons.clone(),
        ),
        CallableFact::Use(row) => base(
            CallableFactKind::Use,
            row.target_symbol_identity.clone(),
            Some(row.target_callable_id.clone()),
            row.exact_eligible,
            row.uncertainty_reasons.clone(),
        ),
        CallableFact::Boundary(row) => base(
            CallableFactKind::Boundary,
            row.subject
                .clone()
                .filter(|subject| full_symbol_identity(subject)),
            None,
            false,
            vec![format!("BOUNDARY_{}", row.code)],
        ),
    }
}

fn build_query_shards(
    binding_digest: &str,
    records: &[CallableIndexRecord],
    budgets: &CallableBudgets,
) -> Result<(Vec<PreparedCasObject>, Vec<CallableQueryShardReference>), ClewError> {
    if records.is_empty() {
        return Err(invalid("callable query index has no records"));
    }
    let mut prepared = Vec::new();
    let mut references = Vec::new();
    let encoded_records = records
        .iter()
        .map(|record| {
            canonical::bytes(record)
                .map(|bytes| bytes.len())
                .map_err(internal)
        })
        .collect::<Result<Vec<_>, _>>()?;
    let encoded_keys = records
        .iter()
        .map(|record| {
            canonical::bytes(&record.key)
                .map(|bytes| bytes.len())
                .map_err(internal)
        })
        .collect::<Result<Vec<_>, _>>()?;
    let mut start = 0usize;
    let mut encoded_rows = 0usize;
    let mut sequence = 0u32;
    for end in 0..records.len() {
        let candidate_rows = encoded_rows
            .checked_add(encoded_records[end])
            .ok_or_else(|| budget("callable query shard size overflowed"))?;
        let candidate_count = end - start + 1;
        if encoded_query_shard_len(
            binding_digest,
            sequence,
            encoded_keys[start],
            encoded_keys[end],
            candidate_rows,
            candidate_count,
        )? <= budgets.max_shard_bytes
        {
            encoded_rows = candidate_rows;
            continue;
        }
        if end == start {
            return Err(budget(
                "one callable query posting exceeds the 8 MiB shard bound",
            ));
        }
        publish_prepared_query_shard(
            binding_digest,
            sequence,
            &records[start..end],
            &mut prepared,
            &mut references,
            budgets,
        )?;
        sequence = sequence
            .checked_add(1)
            .ok_or_else(|| budget("callable query shard sequence overflowed"))?;
        start = end;
        encoded_rows = encoded_records[end];
        if encoded_query_shard_len(
            binding_digest,
            sequence,
            encoded_keys[end],
            encoded_keys[end],
            encoded_rows,
            1,
        )? > budgets.max_shard_bytes
        {
            return Err(budget(
                "one callable query posting exceeds the 8 MiB shard bound",
            ));
        }
    }
    if start < records.len() {
        publish_prepared_query_shard(
            binding_digest,
            sequence,
            &records[start..],
            &mut prepared,
            &mut references,
            budgets,
        )?;
    }
    Ok((prepared, references))
}

fn encoded_query_shard_len(
    binding_digest: &str,
    sequence: u32,
    first_key_len: usize,
    last_key_len: usize,
    encoded_rows: usize,
    row_count: usize,
) -> Result<usize, ClewError> {
    encoded_closed_object_len(&[
        ("bindingDigest", encoded_ascii_string_len(binding_digest)?),
        ("firstKey", first_key_len),
        ("lastKey", last_key_len),
        ("records", encoded_array_len(encoded_rows, row_count)?),
        (
            "schema",
            encoded_ascii_string_len(CALLABLE_QUERY_SHARD_SCHEMA)?,
        ),
        ("sequence", sequence.to_string().len()),
    ])
}

fn callable_query_shard(
    binding_digest: &str,
    sequence: u32,
    records: &[CallableIndexRecord],
) -> Result<CallableQueryShard, ClewError> {
    let first = records
        .first()
        .ok_or_else(|| invalid("callable query shard is empty"))?;
    let last = records.last().expect("non-empty checked above");
    Ok(CallableQueryShard {
        schema: CALLABLE_QUERY_SHARD_SCHEMA.into(),
        binding_digest: binding_digest.into(),
        sequence,
        first_key: first.key.clone(),
        last_key: last.key.clone(),
        records: records.to_vec(),
    })
}

fn publish_prepared_query_shard(
    binding_digest: &str,
    sequence: u32,
    records: &[CallableIndexRecord],
    prepared: &mut Vec<PreparedCasObject>,
    references: &mut Vec<CallableQueryShardReference>,
    budgets: &CallableBudgets,
) -> Result<(), ClewError> {
    let shard = callable_query_shard(binding_digest, sequence, records)?;
    #[cfg(test)]
    QUERY_SHARD_SERIALIZATIONS.with(|count| count.set(count.get() + 1));
    let bytes = canonical::bytes(&shard).map_err(internal)?;
    if bytes.len() > budgets.max_shard_bytes {
        return Err(budget("callable query shard exceeds 8 MiB"));
    }
    let object = CasObject::for_bytes(CALLABLE_QUERY_SHARD_SCHEMA, &bytes)?;
    references.push(CallableQueryShardReference {
        sequence,
        first_key: shard.first_key.clone(),
        last_key: shard.last_key.clone(),
        object: object.clone(),
    });
    prepared.push(PreparedCasObject {
        reference: object,
        bytes,
    });
    Ok(())
}

fn index_key_counts(records: &[CallableIndexRecord]) -> (usize, usize, usize) {
    let mut exact = BTreeSet::new();
    let mut family = BTreeSet::new();
    let mut token = BTreeSet::new();
    for record in records {
        match record.key.kind {
            CallableLookupKind::FullSymbol => &mut exact,
            CallableLookupKind::CallableFamily => &mut family,
            CallableLookupKind::Token => &mut token,
        }
        .insert(record.key.clone());
    }
    (exact.len(), family.len(), token.len())
}

fn query_index_id(index: &CallableQueryIndexManifest) -> Result<String, ClewError> {
    let mut unsigned = index.clone();
    unsigned.index_id.clear();
    canonical::hash(&unsigned).map_err(internal)
}

fn authority_digest(authority: &CallableFactSetAuthority) -> Result<String, ClewError> {
    let mut unsigned = authority.clone();
    unsigned.authority_digest.clear();
    canonical::hash(&unsigned).map_err(internal)
}

fn canonical_cas_closure(
    references: Vec<CasObject>,
    budgets: &CallableBudgets,
) -> Result<Vec<CasObject>, ClewError> {
    let mut unique = BTreeMap::<(String, String), CasObject>::new();
    for reference in references {
        validate_cas_object(&reference)?;
        let key = (reference.object_schema.clone(), reference.digest.clone());
        match unique.get(&key) {
            Some(existing) if existing != &reference => {
                return Err(invalid("direct CAS closure contains conflicting metadata"));
            }
            None => {
                unique.insert(key, reference);
            }
            _ => {}
        }
    }
    let closure = unique.into_values().collect::<Vec<_>>();
    validate_direct_cas_closure_size(&closure, budgets.max_direct_cas_closure_bytes)?;
    Ok(closure)
}

pub(crate) fn validate_direct_cas_closure_size(
    closure: &[CasObject],
    max_bytes: usize,
) -> Result<usize, ClewError> {
    let bytes = closure.iter().try_fold(0usize, |total, reference| {
        validate_cas_object(reference)?;
        let size = usize::try_from(reference.size)
            .map_err(|_| budget("direct CAS closure object exceeds host size"))?;
        total
            .checked_add(size)
            .ok_or_else(|| budget("direct CAS closure size overflowed"))
    })?;
    if bytes > max_bytes {
        let mut bytes_by_schema = BTreeMap::<String, u64>::new();
        for reference in closure {
            *bytes_by_schema
                .entry(reference.object_schema.clone())
                .or_default() += reference.size;
        }
        return Err(budget(format!(
            "direct CAS closure uses {bytes} bytes, exceeding {}-byte limit; bytes by schema: {bytes_by_schema:?}",
            max_bytes
        )));
    }
    Ok(bytes)
}

pub fn query_prepared(
    prepared: &PreparedCallableFactSet,
    request: CallableQueryRequest,
) -> Result<CallableQueryResult, ClewError> {
    verify_prepared(prepared)?;
    query_verified(prepared, request)
}

pub(crate) fn query_verified(
    prepared: &PreparedCallableFactSet,
    request: CallableQueryRequest,
) -> Result<CallableQueryResult, ClewError> {
    query_verified_with_trace(prepared, request).map(|trace| trace.result)
}

pub(crate) fn query_verified_with_trace(
    prepared: &PreparedCallableFactSet,
    mut request: CallableQueryRequest,
) -> Result<CallableQueryTrace, ClewError> {
    if request.lookups.is_empty()
        || request.lookups.len() > MAX_CALLABLE_QUERY_TERMS
        || request.max_results == 0
        || request.max_results > MAX_CALLABLE_QUERY_RESULTS
    {
        return Err(invalid("callable query term or result limit is invalid"));
    }
    for lookup in &mut request.lookups {
        normalize_lookup(lookup)?;
    }
    request.lookups.sort();
    request.lookups.dedup();
    if request.lookups.len() > MAX_CALLABLE_QUERY_TERMS {
        return Err(budget("normalized callable query terms exceed 256"));
    }

    let objects = prepared
        .query_shards
        .iter()
        .map(|object| (object.reference.digest.as_str(), object))
        .collect::<BTreeMap<_, _>>();
    let mut hits = BTreeSet::<(CallableLookup, CallablePosting, LookupAuthority)>::new();
    let mut unmatched = Vec::new();
    let mut shards_read = BTreeMap::<String, CasObject>::new();
    let mut exact_declarations = BTreeMap::<CallableLookup, usize>::new();
    let mut family_symbols = BTreeMap::<CallableLookup, BTreeSet<String>>::new();
    for lookup in &request.lookups {
        let key = index_key_for_lookup(lookup)?;
        let mut matched = false;
        for reference in prepared
            .query_index
            .shards
            .iter()
            .filter(|reference| reference.first_key <= key && key <= reference.last_key)
        {
            let object = objects
                .get(reference.object.digest.as_str())
                .ok_or_else(|| corrupt("query index references an unavailable prepared shard"))?;
            let shard: CallableQueryShard = serde_json::from_slice(&object.bytes)
                .map_err(|_| corrupt("prepared callable query shard is invalid"))?;
            match shards_read.entry(reference.object.digest.clone()) {
                std::collections::btree_map::Entry::Vacant(entry) => {
                    entry.insert(reference.object.clone());
                }
                std::collections::btree_map::Entry::Occupied(entry)
                    if entry.get() != &reference.object =>
                {
                    return Err(corrupt(
                        "query shard digest repeats with conflicting authority",
                    ));
                }
                std::collections::btree_map::Entry::Occupied(_) => {}
            }
            for record in shard.records.iter().filter(|record| record.key == key) {
                matched = true;
                let authority = match lookup {
                    CallableLookup::FullSymbol { .. } if record.posting.exact_eligible => {
                        LookupAuthority::ExactFullSymbol
                    }
                    CallableLookup::FullSymbol { .. } => LookupAuthority::Unsure,
                    CallableLookup::CallableFamily { .. } | CallableLookup::Token { .. }
                        if record.posting.exact_eligible =>
                    {
                        LookupAuthority::NavigationOnly
                    }
                    CallableLookup::CallableFamily { .. } | CallableLookup::Token { .. } => {
                        LookupAuthority::Unsure
                    }
                };
                if matches!(lookup, CallableLookup::FullSymbol { .. })
                    && record.posting.fact_kind == CallableFactKind::Declaration
                    && authority == LookupAuthority::ExactFullSymbol
                {
                    *exact_declarations.entry(lookup.clone()).or_default() += 1;
                }
                if matches!(lookup, CallableLookup::CallableFamily { .. })
                    && record.posting.fact_kind == CallableFactKind::Declaration
                    && let Some(symbol) = &record.posting.symbol_identity
                {
                    family_symbols
                        .entry(lookup.clone())
                        .or_default()
                        .insert(format!(
                            "{}\u{0}{symbol}",
                            record.posting.repository_namespace
                        ));
                }
                hits.insert((lookup.clone(), record.posting.clone(), authority));
            }
        }
        if !matched {
            unmatched.push(lookup.clone());
        }
    }
    let available = hits
        .into_iter()
        .map(|(lookup, posting, authority)| CallableQueryHit {
            lookup,
            posting,
            authority,
        })
        .collect::<Vec<_>>();
    let truncated = available.len() > request.max_results;
    let selected = available
        .iter()
        .take(request.max_results)
        .cloned()
        .collect::<Vec<_>>();
    let matched_postings = available
        .iter()
        .map(|hit| hit.posting.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let ambiguous = family_symbols.values().any(|symbols| symbols.len() > 1);
    let posting_is_unsure = available
        .iter()
        .any(|hit| hit.authority == LookupAuthority::Unsure);
    let unmatched_scope_is_unsure = unmatched
        .iter()
        .any(|lookup| lookup_absence_is_unsure(prepared, lookup));
    let has_unsure = posting_is_unsure || unmatched_scope_is_unsure;
    let all_exact_full_symbols = !request.lookups.is_empty()
        && request
            .lookups
            .iter()
            .all(|lookup| matches!(lookup, CallableLookup::FullSymbol { .. }))
        && unmatched.is_empty()
        && request
            .lookups
            .iter()
            .all(|lookup| exact_declarations.get(lookup) == Some(&1));
    let status = if truncated || has_unsure {
        CallableQueryStatus::Unsure
    } else if available.is_empty() {
        CallableQueryStatus::NotFound
    } else if ambiguous {
        CallableQueryStatus::Ambiguous
    } else if all_exact_full_symbols {
        CallableQueryStatus::ExactFullSymbol
    } else {
        CallableQueryStatus::NavigationOnly
    };
    let mut obligations = BTreeSet::new();
    if truncated {
        obligations.insert("NARROW_OR_EXPAND_QUERY".to_owned());
    }
    if ambiguous {
        obligations.insert("DISAMBIGUATE_OVERLOAD_SET".to_owned());
    }
    if has_unsure {
        obligations.insert("VERIFY_PARTIAL_OR_BOUNDARY_EVIDENCE".to_owned());
    }
    Ok(CallableQueryTrace {
        result: CallableQueryResult {
            schema: "codeclew-kotlin-callable-query-result/1.0".into(),
            index_id: prepared.query_index.index_id.clone(),
            requested_lookups: request.lookups,
            unmatched_lookups: unmatched,
            hits: selected,
            status,
            truncated,
            query_shards_read: shards_read.len(),
            verification_obligations: obligations.into_iter().collect(),
        },
        matched_postings,
        query_shard_refs: shards_read.into_values().collect(),
    })
}

fn lookup_absence_is_unsure(prepared: &PreparedCallableFactSet, lookup: &CallableLookup) -> bool {
    match lookup {
        CallableLookup::FullSymbol {
            repository_namespace,
            ..
        }
        | CallableLookup::CallableFamily {
            repository_namespace: Some(repository_namespace),
            ..
        } => !descriptor_scope_is_complete(prepared, Some(repository_namespace)),
        CallableLookup::CallableFamily {
            repository_namespace: None,
            ..
        } => !descriptor_scope_is_complete(prepared, None),
        CallableLookup::Token { .. } => {
            prepared.authority.members.is_empty()
                || prepared
                    .authority
                    .members
                    .iter()
                    .flat_map(|member| &member.compilations)
                    .any(|compilation| {
                        compilation.descriptor_coverage == GraphCoverage::Partial
                            || compilation.relation_coverage == GraphCoverage::Partial
                    })
        }
    }
}

fn descriptor_scope_is_complete(
    prepared: &PreparedCallableFactSet,
    repository_namespace: Option<&str>,
) -> bool {
    let mut selected = 0usize;
    for member in &prepared.authority.members {
        if repository_namespace.is_some_and(|namespace| namespace != member.repository_namespace) {
            continue;
        }
        selected += 1;
        if member.compilations.is_empty()
            || member
                .compilations
                .iter()
                .any(|compilation| compilation.descriptor_coverage == GraphCoverage::Partial)
        {
            return false;
        }
    }
    selected > 0
}

pub fn verify_prepared(prepared: &PreparedCallableFactSet) -> Result<(), ClewError> {
    verify_authority_projection(&prepared.authority, &prepared.projection)?;
    if canonical::bytes(&prepared.authority).map_err(internal)? != prepared.authority_bytes
        || canonical::bytes(&prepared.projection).map_err(internal)? != prepared.projection_bytes
        || prepared.projection_bytes.len().saturating_add(1)
            > prepared.authority.budgets.max_stdout_bytes
    {
        return Err(corrupt(
            "prepared callable authority or projection is inconsistent",
        ));
    }
    verify_prepared_object(
        &prepared.query_index_object,
        CALLABLE_QUERY_INDEX_SCHEMA,
        prepared.authority.budgets.max_shard_bytes,
    )?;
    verify_prepared_object(
        &prepared.evidence_object,
        CALLABLE_FACT_SET_EVIDENCE_SCHEMA,
        MAX_CALLABLE_EVIDENCE_OBJECT_BYTES,
    )?;
    if canonical::bytes(&prepared.query_index).map_err(internal)?
        != prepared.query_index_object.bytes
        || query_index_id(&prepared.query_index)? != prepared.query_index.index_id
        || prepared.query_index.binding_digest != prepared.authority.binding_digest
        || prepared.query_index_object.reference != prepared.authority.query_index_ref
        || canonical::bytes(&prepared.evidence).map_err(internal)? != prepared.evidence_object.bytes
        || prepared.evidence_object.reference != prepared.authority.evidence_ref
        || prepared.evidence.binding_digest != prepared.authority.binding_digest
        || prepared.evidence.query_index_ref != prepared.authority.query_index_ref
        || prepared.evidence.fact_shards != prepared.authority.fact_shards
        || prepared.evidence.counts != prepared.authority.counts
        || prepared.evidence.completeness != prepared.authority.completeness
        || prepared.evidence.members != prepared.authority.members
        || prepared.query_index.fact_shards != prepared.authority.fact_shards
    {
        return Err(corrupt(
            "prepared callable index/evidence binding is inconsistent",
        ));
    }

    if prepared.fact_shards.len() != prepared.authority.fact_shards.len()
        || prepared.query_shards.len() != prepared.query_index.shards.len()
    {
        return Err(corrupt("prepared callable shard set is incomplete"));
    }
    let mut previous_fact = None::<String>;
    let mut verified_facts = Vec::new();
    for (expected, object) in prepared
        .authority
        .fact_shards
        .iter()
        .zip(&prepared.fact_shards)
    {
        verify_prepared_object(
            object,
            CALLABLE_FACT_SHARD_SCHEMA,
            prepared.authority.budgets.max_shard_bytes,
        )?;
        let shard: CallableFactShard = serde_json::from_slice(&object.bytes)
            .map_err(|_| corrupt("prepared callable fact shard is invalid"))?;
        if expected.object != object.reference
            || expected.sequence != shard.sequence
            || expected.first_fact_id != shard.first_fact_id
            || expected.last_fact_id != shard.last_fact_id
            || shard.schema != CALLABLE_FACT_SHARD_SCHEMA
            || shard.facts.is_empty()
            || shard.first_fact_id != shard.facts.first().unwrap().fact_id()
            || shard.last_fact_id != shard.facts.last().unwrap().fact_id()
        {
            return Err(corrupt("prepared callable fact shard authority is invalid"));
        }
        for fact in &shard.facts {
            if previous_fact
                .as_deref()
                .is_some_and(|previous| previous >= fact.fact_id())
                || !fact_id_valid(fact)?
            {
                return Err(corrupt(
                    "prepared callable fact order or identity is invalid",
                ));
            }
            previous_fact = Some(fact.fact_id().to_owned());
            verified_facts.push(fact.clone());
        }
    }
    let mut previous_record = None::<CallableIndexRecord>;
    let mut verified_records = Vec::new();
    for (expected, object) in prepared
        .query_index
        .shards
        .iter()
        .zip(&prepared.query_shards)
    {
        verify_prepared_object(
            object,
            CALLABLE_QUERY_SHARD_SCHEMA,
            prepared.authority.budgets.max_shard_bytes,
        )?;
        let shard: CallableQueryShard = serde_json::from_slice(&object.bytes)
            .map_err(|_| corrupt("prepared callable query shard is invalid"))?;
        if expected.object != object.reference
            || expected.sequence != shard.sequence
            || expected.first_key != shard.first_key
            || expected.last_key != shard.last_key
            || shard.schema != CALLABLE_QUERY_SHARD_SCHEMA
            || shard.binding_digest != prepared.authority.binding_digest
            || shard.records.is_empty()
            || shard.first_key != shard.records.first().unwrap().key
            || shard.last_key != shard.records.last().unwrap().key
        {
            return Err(corrupt(
                "prepared callable query shard authority is invalid",
            ));
        }
        for record in &shard.records {
            if previous_record
                .as_ref()
                .is_some_and(|previous| previous >= record)
            {
                return Err(corrupt("prepared callable query posting order is invalid"));
            }
            previous_record = Some(record.clone());
            verified_records.push(record.clone());
        }
    }
    let declarations = verified_facts
        .iter()
        .filter_map(|fact| match fact {
            CallableFact::Declaration(row) => Some(row.clone()),
            _ => None,
        })
        .collect::<Vec<_>>();
    reject_duplicate_declarations(&declarations)
        .map_err(|_| corrupt("prepared callable declarations repeat an identity"))?;
    let uses = verified_facts
        .iter()
        .filter(|fact| matches!(fact, CallableFact::Use(_)))
        .count();
    let boundary_ids = verified_facts
        .iter()
        .filter_map(|fact| match fact {
            CallableFact::Boundary(row) => Some(row.fact_id.clone()),
            _ => None,
        })
        .collect::<Vec<_>>();
    let actual_counts = CallableFactCounts {
        visited_input_facts: prepared.authority.counts.visited_input_facts,
        visited_input_payload_bytes: prepared.authority.counts.visited_input_payload_bytes,
        declarations: declarations.len(),
        uses,
        boundaries: boundary_ids.len(),
        total: verified_facts.len(),
        exact_declarations: declarations.iter().filter(|row| row.exact_eligible).count(),
        exact_uses: verified_facts
            .iter()
            .filter(|fact| matches!(fact, CallableFact::Use(row) if row.exact_eligible))
            .count(),
    };
    let has_partial_compilation = prepared
        .authority
        .members
        .iter()
        .flat_map(|member| &member.compilations)
        .any(|compilation| {
            compilation.descriptor_coverage == GraphCoverage::Partial
                || compilation.relation_coverage == GraphCoverage::Partial
        });
    let actual_completeness = CallableCompleteness {
        coverage: if boundary_ids.is_empty() && !has_partial_compilation {
            CallableFactSetCoverage::Complete
        } else {
            CallableFactSetCoverage::Partial
        },
        certainty: if boundary_ids.is_empty() && !has_partial_compilation {
            CallableFactSetCertainty::Verified
        } else {
            CallableFactSetCertainty::Unsure
        },
        obligation_count: boundary_ids.len(),
    };
    let reconstructed_records = build_index_records(&verified_facts)
        .map_err(|_| corrupt("prepared callable query postings cannot be reconstructed"))?;
    let key_counts = index_key_counts(&verified_records);
    let mut payload_bindings = verified_facts
        .iter()
        .map(|fact| {
            let provenance = fact.provenance();
            InputPayloadBinding {
                member_alias: provenance.member_alias.clone(),
                compilation_id: provenance.compilation_id.clone(),
                fact_key: provenance.input_fact_key.clone(),
                payload_ref: provenance.input_payload_ref.clone(),
                source_ref: provenance
                    .source
                    .as_ref()
                    .map(|source| source.content_ref.clone()),
            }
        })
        .collect::<Vec<_>>();
    payload_bindings.sort_by(|left, right| {
        (
            left.member_alias.as_str(),
            left.compilation_id.as_str(),
            left.fact_key.as_str(),
            left.payload_ref.digest.as_str(),
        )
            .cmp(&(
                right.member_alias.as_str(),
                right.compilation_id.as_str(),
                right.fact_key.as_str(),
                right.payload_ref.digest.as_str(),
            ))
    });
    let reconstructed_request = CallableFactSetRequest {
        thread_id: prepared.authority.thread_id.clone(),
        thread_authority_digest: prepared.authority.thread_authority_digest.clone(),
        thread_context_id: prepared.authority.thread_context_id.clone(),
        thread_context_authority_digest: prepared.authority.thread_context_authority_digest.clone(),
        profile_digest: prepared.authority.profile_digest.clone(),
        tasks: prepared.authority.tasks.clone(),
        pairs: prepared.authority.pairs.clone(),
        budgets: prepared.authority.budgets.clone(),
    };
    let reconstructed_binding_digest = canonical::hash(&CallableBindingMaterial {
        schema: "codeclew-kotlin-callable-binding/1.0",
        request: &reconstructed_request,
        input_visit: InputVisitBinding {
            fact_count: prepared.authority.counts.visited_input_facts,
            payload_bytes: prepared.authority.counts.visited_input_payload_bytes,
        },
        members: &prepared.authority.members,
        payloads: payload_bindings,
    })
    .map_err(internal)?;
    if actual_counts != prepared.authority.counts
        || actual_completeness != prepared.authority.completeness
        || boundary_ids != prepared.evidence.boundary_fact_ids
        || reconstructed_records != verified_records
        || reconstructed_binding_digest != prepared.authority.binding_digest
        || prepared.query_index.exact_symbol_key_count != key_counts.0
        || prepared.query_index.callable_family_key_count != key_counts.1
        || prepared.query_index.token_key_count != key_counts.2
        || prepared.query_index.posting_count != verified_records.len()
    {
        return Err(corrupt(
            "prepared callable evidence/index content was substituted",
        ));
    }
    let mut required_closure = Vec::new();
    for member in &prepared.authority.members {
        required_closure.push(member.snapshot_ref.clone());
        required_closure.extend(
            member
                .compilations
                .iter()
                .map(|compilation| compilation.generation_ref.clone()),
        );
    }
    for fact in &verified_facts {
        let provenance = fact.provenance();
        required_closure.push(provenance.input_payload_ref.clone());
        if let Some(source) = &provenance.source {
            required_closure.push(source.content_ref.clone());
        }
    }
    required_closure.extend(
        prepared
            .authority
            .pairs
            .iter()
            .filter_map(|pair| pair.dependency_evidence_ref.clone()),
    );
    required_closure.extend(
        prepared
            .fact_shards
            .iter()
            .map(|object| object.reference.clone()),
    );
    required_closure.extend(
        prepared
            .query_shards
            .iter()
            .map(|object| object.reference.clone()),
    );
    required_closure.extend([
        prepared.authority.query_index_ref.clone(),
        prepared.authority.evidence_ref.clone(),
    ]);
    let expected_closure = canonical_cas_closure(required_closure, &prepared.authority.budgets)?;
    if expected_closure != prepared.authority.direct_cas_closure {
        return Err(corrupt("prepared callable direct CAS closure is invalid"));
    }
    let derived_count = prepared.fact_shards.len() + prepared.query_shards.len() + 2;
    if derived_count > prepared.authority.budgets.max_derived_cas_objects {
        return Err(corrupt(
            "prepared callable derived CAS object count is invalid",
        ));
    }
    Ok(())
}

fn verify_prepared_object(
    object: &PreparedCasObject,
    schema: &str,
    max_bytes: usize,
) -> Result<(), ClewError> {
    if object.bytes.len() > max_bytes
        || object.reference != CasObject::for_bytes(schema, &object.bytes)?
    {
        return Err(corrupt(
            "prepared callable CAS bytes differ from their predicted identity",
        ));
    }
    Ok(())
}

fn fact_id_valid(fact: &CallableFact) -> Result<bool, ClewError> {
    Ok(match fact {
        CallableFact::Declaration(row) => declaration_fact_id(row)? == row.fact_id,
        CallableFact::Use(row) => use_fact_id(row)? == row.fact_id,
        CallableFact::Boundary(row) => boundary_fact_id(row)? == row.fact_id,
    })
}

fn normalize_lookup(lookup: &mut CallableLookup) -> Result<(), ClewError> {
    match lookup {
        CallableLookup::FullSymbol {
            repository_namespace,
            symbol_identity,
        } => {
            validate_repository_namespace(repository_namespace)?;
            validate_text(symbol_identity, "full symbol identity")?;
            if !full_symbol_identity(symbol_identity) {
                return Err(invalid(
                    "exact callable lookup is not a full symbol identity",
                ));
            }
        }
        CallableLookup::CallableFamily {
            repository_namespace,
            callable_id,
        } => {
            if let Some(namespace) = repository_namespace {
                validate_repository_namespace(namespace)?;
            }
            validate_text(callable_id, "CallableId family")?;
        }
        CallableLookup::Token { term } => {
            validate_text(term, "identifier token")?;
            if !term
                .chars()
                .all(|character| character.is_alphanumeric() || character == '_')
            {
                return Err(invalid("identifier token must be one bounded token"));
            }
            *term = term.chars().flat_map(char::to_lowercase).collect();
            if term.len() < 2 || term.len() > 256 {
                return Err(invalid("identifier token length is invalid"));
            }
        }
    }
    Ok(())
}

fn index_key_for_lookup(lookup: &CallableLookup) -> Result<CallableIndexKey, ClewError> {
    Ok(match lookup {
        CallableLookup::FullSymbol {
            repository_namespace,
            symbol_identity,
        } => CallableIndexKey {
            kind: CallableLookupKind::FullSymbol,
            repository_namespace: Some(repository_namespace.clone()),
            value: symbol_identity.clone(),
        },
        CallableLookup::CallableFamily {
            repository_namespace,
            callable_id,
        } => CallableIndexKey {
            kind: CallableLookupKind::CallableFamily,
            repository_namespace: repository_namespace.clone(),
            value: callable_id.clone(),
        },
        CallableLookup::Token { term } => CallableIndexKey {
            kind: CallableLookupKind::Token,
            repository_namespace: None,
            value: term.clone(),
        },
    })
}

fn payload_schema(input: &QualifiedCallablePayload) -> Option<&str> {
    input.payload.get("schema").and_then(Value::as_str)
}

fn payload_category(schema: &str) -> &'static str {
    match schema {
        "declaration-descriptor/0.1" => "descriptor",
        "declaration-relation/0.1" => "relation",
        "declaration-descriptor-boundary/0.1" => "descriptor-boundary",
        "declaration-relation-boundary/0.1" => "relation-boundary",
        _ => "unsupported",
    }
}

fn required_string<'a>(value: &'a Value, field: &str) -> Result<&'a str, ClewError> {
    value
        .get(field)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| invalid(format!("qualified Kotlin payload has no {field}")))
}

fn required_bounded_string(
    value: &Value,
    field: &str,
    budgets: &CallableBudgets,
) -> Result<String, ClewError> {
    let value = required_string(value, field)?;
    if value.len() > budgets.max_text_bytes {
        return Err(budget(format!("{field} exceeds 4,096 bytes")));
    }
    Ok(value.to_owned())
}

fn optional_bounded_string(
    value: &Value,
    field: &str,
    budgets: &CallableBudgets,
) -> Result<Option<String>, ClewError> {
    let Some(value) = value.get(field) else {
        return Ok(None);
    };
    let value = value
        .as_str()
        .filter(|value| !value.is_empty())
        .ok_or_else(|| invalid(format!("qualified Kotlin {field} is not a string")))?;
    if value.len() > budgets.max_text_bytes {
        return Err(budget(format!("{field} exceeds 4,096 bytes")));
    }
    Ok(Some(value.to_owned()))
}

fn string_array(value: &Value, field: &str, limit: usize) -> Result<Vec<String>, ClewError> {
    let values = value
        .get(field)
        .and_then(Value::as_array)
        .ok_or_else(|| invalid(format!("qualified Kotlin payload has no {field} array")))?;
    if values.len() > limit {
        return Err(budget(format!(
            "qualified Kotlin {field} exceeds its bound"
        )));
    }
    values
        .iter()
        .map(|value| {
            let value = value
                .as_str()
                .filter(|value| !value.is_empty())
                .ok_or_else(|| invalid(format!("qualified Kotlin {field} is not textual")))?;
            validate_text(value, field)?;
            Ok(value.to_owned())
        })
        .collect()
}

fn validate_json_strings(value: &Value, max_bytes: usize, depth: usize) -> Result<(), ClewError> {
    if depth > 64 {
        return Err(budget("qualified Kotlin payload nesting exceeds 64"));
    }
    match value {
        Value::String(value) => {
            if value.len() > max_bytes || !crate::text_authority::is_nfc(value) {
                return Err(budget(
                    "qualified Kotlin identifier/type/path exceeds text authority",
                ));
            }
        }
        Value::Array(values) => {
            for value in values {
                validate_json_strings(value, max_bytes, depth + 1)?;
            }
        }
        Value::Object(values) => {
            for (key, value) in values {
                if key.len() > max_bytes || !crate::text_authority::is_nfc(key) {
                    return Err(budget(
                        "qualified Kotlin payload key exceeds text authority",
                    ));
                }
                validate_json_strings(value, max_bytes, depth + 1)?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn validate_text(value: &str, label: &str) -> Result<(), ClewError> {
    if value.is_empty()
        || value.len() > MAX_CALLABLE_TEXT_BYTES
        || !crate::text_authority::is_nfc(value)
        || value.chars().any(char::is_control)
    {
        return Err(invalid(format!(
            "{label} is empty or outside text authority"
        )));
    }
    Ok(())
}

fn validate_alias(value: &str, label: &str) -> Result<(), ClewError> {
    if value.is_empty()
        || value.len() > 64
        || !crate::text_authority::is_nfc(value)
        || !value.bytes().enumerate().all(|(index, byte)| {
            byte.is_ascii_alphanumeric() || (index > 0 && matches!(byte, b'-' | b'_' | b'.'))
        })
    {
        return Err(invalid(format!("{label} is not a safe bounded alias")));
    }
    Ok(())
}

fn validate_digest(value: &str, label: &str) -> Result<(), ClewError> {
    if !is_digest(value) {
        return Err(invalid(format!(
            "{label} is not a canonical SHA-256 digest"
        )));
    }
    Ok(())
}

fn is_digest(value: &str) -> bool {
    value.strip_prefix("sha256:").is_some_and(|digest| {
        digest.len() == 64
            && digest
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    })
}

fn validate_cas_object(object: &CasObject) -> Result<(), ClewError> {
    if object.schema != CAS_OBJECT_SCHEMA || !is_digest(&object.digest) {
        return Err(invalid("CAS reference identity is invalid"));
    }
    validate_text(&object.object_schema, "CAS object schema")
}

fn validate_relative_path(path: &str) -> Result<(), ClewError> {
    validate_text(path, "repository-relative source path")?;
    if Path::new(path).is_absolute()
        || !Path::new(path)
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
        || path.contains("://")
    {
        return Err(invalid("source path is not repository-relative"));
    }
    Ok(())
}

fn validate_repository_namespace(namespace: &str) -> Result<(), ClewError> {
    if namespace.strip_prefix("repo:").is_none_or(|digest| {
        digest.len() != 64
            || !digest
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    }) {
        return Err(invalid("repository namespace is invalid"));
    }
    Ok(())
}

fn full_symbol_identity(value: &str) -> bool {
    crate::semantic_validation::validate_kotlin_full_symbol_identity(value).is_ok()
}

fn callable_family_from_symbol(value: &str) -> Option<String> {
    for prefix in ["callable:", "constructor:"] {
        if let Some(value) = value.strip_prefix(prefix) {
            return value
                .split_once("#jvm:")
                .map(|(callable, _)| callable.to_owned());
        }
    }
    value
        .strip_prefix("property:")
        .or_else(|| value.strip_prefix("class:"))
        .map(str::to_owned)
}

fn collect_json_index_strings(
    value: &Value,
    output: &mut BTreeSet<String>,
    depth: usize,
) -> Result<(), ClewError> {
    if depth > 64 {
        return Err(budget("callable index metadata nesting exceeds 64"));
    }
    match value {
        Value::String(value) if !is_digest(value) => {
            output.insert(value.clone());
        }
        Value::Array(values) => {
            for value in values {
                collect_json_index_strings(value, output, depth + 1)?;
            }
        }
        Value::Object(values) => {
            for value in values.values() {
                collect_json_index_strings(value, output, depth + 1)?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn identifier_terms(value: &str) -> Result<BTreeSet<String>, ClewError> {
    let mut output = BTreeSet::new();
    let mut token = String::new();
    let flush = |token: &mut String, output: &mut BTreeSet<String>| {
        if token.len() >= 2 && token.len() <= 256 {
            output.insert(token.chars().flat_map(char::to_lowercase).collect());
            split_identifier(token, output);
        }
        token.clear();
    };
    for character in value.chars() {
        if character.is_alphanumeric() || character == '_' {
            token.push(character);
            if token.len() > 256 {
                token.clear();
            }
        } else {
            flush(&mut token, &mut output);
        }
    }
    flush(&mut token, &mut output);
    Ok(output)
}

fn split_identifier(token: &str, output: &mut BTreeSet<String>) {
    for component in token.split('_') {
        let characters = component.chars().collect::<Vec<_>>();
        if characters.is_empty() {
            continue;
        }
        let mut start = 0usize;
        for index in 1..characters.len() {
            let previous = characters[index - 1];
            let current = characters[index];
            let next = characters.get(index + 1).copied();
            if current.is_uppercase()
                && (previous.is_lowercase()
                    || previous.is_numeric()
                    || (previous.is_uppercase() && next.is_some_and(char::is_lowercase)))
            {
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

fn encoded_array_len(encoded_items: usize, item_count: usize) -> Result<usize, ClewError> {
    encoded_items
        .checked_add(2)
        .and_then(|value| value.checked_add(item_count.saturating_sub(1)))
        .ok_or_else(|| budget("canonical array size overflowed"))
}

fn encoded_ascii_string_len(value: &str) -> Result<usize, ClewError> {
    if !value
        .bytes()
        .all(|byte| byte.is_ascii() && !byte.is_ascii_control() && !matches!(byte, b'"' | b'\\'))
    {
        return Err(invalid(
            "canonical shard header contains a non-literal-safe identity",
        ));
    }
    value
        .len()
        .checked_add(2)
        .ok_or_else(|| budget("canonical string size overflowed"))
}

fn encoded_closed_object_len(fields: &[(&str, usize)]) -> Result<usize, ClewError> {
    let mut length = 2usize;
    for (index, (key, value_len)) in fields.iter().enumerate() {
        let key_len = encoded_ascii_string_len(key)?;
        length = length
            .checked_add(usize::from(index > 0))
            .and_then(|value| value.checked_add(key_len))
            .and_then(|value| value.checked_add(1))
            .and_then(|value| value.checked_add(*value_len))
            .ok_or_else(|| budget("canonical object size overflowed"))?;
    }
    Ok(length)
}

fn invalid(message: impl Into<String>) -> ClewError {
    ClewError::new(ErrorCode::InvalidInput, message)
}

fn budget(message: impl Into<String>) -> ClewError {
    ClewError::new(ErrorCode::SliceBudgetExceeded, message)
}

fn corrupt(message: impl Into<String>) -> ClewError {
    ClewError::new(ErrorCode::StateCorrupt, message)
}

fn internal(error: impl std::fmt::Display) -> ClewError {
    ClewError::new(ErrorCode::Internal, error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    fn digest(label: &str) -> String {
        canonical::hash(&label).unwrap()
    }

    fn object(schema: &str, label: &str) -> CasObject {
        CasObject::for_bytes(schema, label.as_bytes()).unwrap()
    }

    fn assert_exact_limit<T>(
        label: &str,
        limit: usize,
        mut validate: impl FnMut(usize) -> Result<T, ClewError>,
    ) {
        assert!(validate(limit).is_ok(), "{label} rejected its exact limit");
        assert!(validate(limit + 1).is_err(), "{label} accepted limit+1");
    }

    fn member(alias: &str) -> CallableMemberAuthority {
        CallableMemberAuthority {
            member_alias: alias.into(),
            service_alias: format!("{alias}-service"),
            session_id: format!("session:{alias}"),
            session_authority_digest: digest(&format!("session-{alias}")),
            repository_key: format!("repository-{alias}"),
            base_revision: digest(&format!("revision-{alias}")),
            snapshot_ref: object(
                "codeclew-repository-input-snapshot/1.0",
                &format!("snapshot-{alias}"),
            ),
        }
    }

    fn compilation(
        alias: &str,
        descriptor_coverage: GraphCoverage,
        relation_coverage: GraphCoverage,
    ) -> CallableCompilationAuthority {
        CallableCompilationAuthority {
            compilation_id: ":app/main".into(),
            generation_id: digest(&format!("generation-id-{alias}")),
            generation_ref: object(
                "codeclew-generation-manifest/2.0",
                &format!("generation-{alias}"),
            ),
            semantic_authority: "K2_FIR".into(),
            extractor_id: "fir-facts-extractor/0.6".into(),
            adapter_digest: digest("adapter"),
            runtime_digest: digest("runtime"),
            descriptor_coverage,
            relation_coverage,
        }
    }

    fn descriptor(callable: &str, jvm: &str, file: &str, start: u64) -> Value {
        json!({
            "schema":"declaration-descriptor/0.1",
            "file":file,
            "start":start,
            "end":start + 8,
            "symbolIdentity":format!("callable:{callable}#jvm:{jvm}"),
            "declarationKind":"FUNCTION",
            "ownerIdentity":format!("class:{}", callable.rsplit_once('.').unwrap().0),
            "containment":[format!("class:{}", callable.rsplit_once('.').unwrap().0)],
            "visibility":"public",
            "effectiveVisibility":"public",
            "exportBoundary":"PUBLIC_API",
            "modality":"FINAL",
            "resolution":"PROVEN",
            "provider":"K2_FIR",
            "module":":app",
            "sourceSet":"main",
            "sourceProvenance":"COMPILER_UTF16_RANGE_TO_UTF8_BYTES",
            "compilerAuthority":"fir-facts-extractor/0.6",
            "typeParameters":[],
            "compilerCallableId":callable,
            "isOverride":false,
            "returnType":"kotlin/String",
            "returnNullable":false,
            "parameterTypes":[],
        })
    }

    fn relation(owner: &str, target: &str, file: &str, start: u64) -> Value {
        json!({
            "schema":"declaration-relation/0.1",
            "file":file,
            "start":start,
            "end":start + 6,
            "kind":"CALLS",
            "owner":owner,
            "target":target,
            "resolution":"PROVEN",
            "provider":"K2_FIR",
            "cfgNodeIds":[],
            "sourceProvenance":"COMPILER_UTF16_RANGE_TO_UTF8_BYTES",
            "orderProvenance":"FIR_SOURCE_RANGE",
        })
    }

    fn boundary(subject: &str, file: &str, start: u64) -> Value {
        json!({
            "schema":"declaration-descriptor-boundary/0.1",
            "file":file,
            "start":start,
            "end":start + 1,
            "stage":"DECLARATION",
            "code":"UNRESOLVED_DESCRIPTOR_TYPE",
            "symbolIdentity":subject,
            "resolution":"UNKNOWN",
            "provider":"K2_FIR",
            "module":"main",
            "sourceSet":"main",
            "sourceProvenance":"COMPILER_UTF16_RANGE_TO_UTF8_BYTES",
            "compilerAuthority":"fir-facts-extractor/0.6",
        })
    }

    fn subjectless_descriptor_boundary(file: &str, start: u64) -> Value {
        json!({
            "schema":"declaration-descriptor-boundary/0.1",
            "file":file,
            "start":start,
            "end":start + 1,
            "stage":"DECLARATION",
            "code":"NO_COMPILER_CALLABLE_ID",
            "resolution":"UNKNOWN",
            "provider":"K2_FIR",
            "module":"main",
            "sourceSet":"main",
            "sourceProvenance":"COMPILER_UTF16_RANGE_TO_UTF8_BYTES",
            "compilerAuthority":"fir-facts-extractor/0.6",
        })
    }

    fn relation_boundary(owner: &str, file: &str, start: u64) -> Value {
        json!({
            "schema":"declaration-relation-boundary/0.1",
            "file":file,
            "start":start,
            "end":start + 1,
            "owner":owner,
            "stage":"CALL_RESOLUTION",
            "code":"UNRESOLVED_RELATION_TARGET",
            "resolution":"UNKNOWN",
            "provider":"K2_FIR",
        })
    }

    fn qualified(
        member: CallableMemberAuthority,
        compilation: CallableCompilationAuthority,
        payload: Value,
    ) -> QualifiedCallablePayload {
        let schema = payload.get("schema").unwrap().as_str().unwrap();
        let bytes = canonical::bytes(&payload).unwrap();
        let hash = canonical::hash_bytes(&bytes);
        let source_ref = payload.get("file").and_then(Value::as_str).map(|file| {
            object(
                "codeclew-repository-source-content/1.0",
                &format!("{}:{file}", member.member_alias),
            )
        });
        QualifiedCallablePayload {
            member,
            compilation,
            fact_key: format!(
                "kotlin:{}:{}",
                payload_category(schema),
                hash.strip_prefix("sha256:").unwrap()
            ),
            payload_ref: CasObject::for_bytes(KOTLIN_SEMANTIC_FACT_SCHEMA, &bytes).unwrap(),
            source_ref,
            payload,
        }
    }

    fn request(authority: RelationshipAuthority) -> CallableFactSetRequest {
        CallableFactSetRequest {
            thread_id: "thread:test".into(),
            thread_authority_digest: digest("thread"),
            thread_context_id: "thread-context:test".into(),
            thread_context_authority_digest: digest("context"),
            profile_digest: digest("profile"),
            tasks: vec![CallableTaskBinding {
                task_id: "task-one".into(),
                pair_id: "pair-one".into(),
                terms: vec!["FindOrder".into()],
            }],
            pairs: vec![CallablePairBinding {
                pair_id: "pair-one".into(),
                provider_member: "left".into(),
                consumer_member: "right".into(),
                relationship_authority: authority,
                dependency_evidence_ref: (authority
                    == RelationshipAuthority::VerifiedSameSnapshotCompilationDependency)
                    .then(|| object("codeclew-compilation-dependency-evidence/1.0", "dependency")),
            }],
            budgets: CallableBudgets::frozen(),
        }
    }

    fn input(payloads: Vec<QualifiedCallablePayload>) -> CallableBuildInput {
        let visited_payload_bytes = payloads
            .iter()
            .map(|payload| canonical::bytes(&payload.payload).unwrap().len())
            .sum();
        let selected_compilations = payloads
            .iter()
            .map(|payload| {
                (
                    (
                        payload.member.member_alias.clone(),
                        payload.compilation.compilation_id.clone(),
                    ),
                    CallableSelectedCompilation {
                        member: payload.member.clone(),
                        compilation: payload.compilation.clone(),
                    },
                )
            })
            .collect::<BTreeMap<_, _>>()
            .into_values()
            .collect();
        CallableBuildInput {
            visited_fact_count: payloads.len(),
            visited_payload_bytes,
            selected_compilations,
            payloads,
        }
    }

    fn fixture(
        left_callable: &str,
        right_callable: &str,
    ) -> (CallableFactSetRequest, CallableBuildInput) {
        let left = member("left");
        let right = member("right");
        let left_compilation = compilation(
            "left",
            GraphCoverage::CompleteSupportedSubset,
            GraphCoverage::CompleteSupportedSubset,
        );
        let right_compilation = compilation(
            "right",
            GraphCoverage::CompleteSupportedSubset,
            GraphCoverage::CompleteSupportedSubset,
        );
        (
            request(RelationshipAuthority::DeclaredTopology),
            input(vec![
                qualified(
                    left,
                    left_compilation,
                    descriptor(left_callable, "()Ljava/lang/String;", "src/Left.kt", 0),
                ),
                qualified(
                    right,
                    right_compilation,
                    descriptor(right_callable, "()Ljava/lang/String;", "src/Right.kt", 10),
                ),
            ]),
        )
    }

    fn facts(prepared: &PreparedCallableFactSet) -> Vec<CallableFact> {
        prepared
            .fact_shards
            .iter()
            .flat_map(|object| {
                serde_json::from_slice::<CallableFactShard>(&object.bytes)
                    .unwrap()
                    .facts
            })
            .collect()
    }

    fn resign(prepared: &mut PreparedCallableFactSet) {
        prepared.query_index.index_id = query_index_id(&prepared.query_index).unwrap();
        let query_index_bytes = canonical::bytes(&prepared.query_index).unwrap();
        let query_index_ref =
            CasObject::for_bytes(CALLABLE_QUERY_INDEX_SCHEMA, &query_index_bytes).unwrap();
        prepared.query_index_object = PreparedCasObject {
            reference: query_index_ref.clone(),
            bytes: query_index_bytes,
        };
        prepared.evidence.query_index_ref = query_index_ref.clone();
        let evidence_bytes = canonical::bytes(&prepared.evidence).unwrap();
        let evidence_ref =
            CasObject::for_bytes(CALLABLE_FACT_SET_EVIDENCE_SCHEMA, &evidence_bytes).unwrap();
        prepared.evidence_object = PreparedCasObject {
            reference: evidence_ref.clone(),
            bytes: evidence_bytes,
        };
        let mut closure = prepared
            .authority
            .direct_cas_closure
            .iter()
            .filter(|reference| {
                !matches!(
                    reference.object_schema.as_str(),
                    CALLABLE_FACT_SHARD_SCHEMA
                        | CALLABLE_QUERY_SHARD_SCHEMA
                        | CALLABLE_QUERY_INDEX_SCHEMA
                        | CALLABLE_FACT_SET_EVIDENCE_SCHEMA
                )
            })
            .cloned()
            .collect::<Vec<_>>();
        closure.extend(
            prepared
                .fact_shards
                .iter()
                .map(|object| object.reference.clone()),
        );
        closure.extend(
            prepared
                .query_shards
                .iter()
                .map(|object| object.reference.clone()),
        );
        closure.extend([query_index_ref.clone(), evidence_ref.clone()]);
        prepared.authority.query_index_ref = query_index_ref.clone();
        prepared.authority.evidence_ref = evidence_ref.clone();
        prepared.authority.direct_cas_closure =
            canonical_cas_closure(closure, &prepared.authority.budgets).unwrap();
        prepared.authority.authority_digest = authority_digest(&prepared.authority).unwrap();
        prepared.authority_bytes = canonical::bytes(&prepared.authority).unwrap();
        prepared.projection = projection_from_authority(&prepared.authority);
        prepared.projection_bytes = canonical::bytes(&prepared.projection).unwrap();
    }

    #[test]
    fn input_task_and_jobs_permutations_are_identity_neutral() {
        let (mut first_request, first_input) = fixture("p/Orders.findOrder", "p/Consumer.call");
        first_request.tasks.push(CallableTaskBinding {
            task_id: "task-two".into(),
            pair_id: "pair-one".into(),
            terms: vec!["Consumer".into(), "Call".into()],
        });
        let mut second_request = first_request.clone();
        second_request.tasks.reverse();
        second_request.tasks[0].terms.reverse();
        let mut second_input = first_input.clone();
        second_input.payloads.reverse();
        second_input.selected_compilations.reverse();

        let one = build_with_jobs(first_request, first_input, 1).unwrap();
        let many = build_with_jobs(second_request, second_input, 16).unwrap();
        assert_eq!(
            one.authority.authority_digest,
            many.authority.authority_digest
        );
        assert_eq!(one.authority_bytes, many.authority_bytes);
        assert_eq!(one.evidence_object, many.evidence_object);
        assert_eq!(one.query_index_object, many.query_index_object);
        assert_eq!(one.fact_shards, many.fact_shards);
        assert_eq!(one.query_shards, many.query_shards);
    }

    #[test]
    fn empty_ready_compilations_are_bound_and_count_toward_the_global_64_limit() {
        let (request, mut exact_input) = fixture("p/Orders.findOrder", "p/Consumer.call");
        for index in exact_input.selected_compilations.len()..MAX_CALLABLE_COMPILATIONS {
            let mut authority = compilation(
                "left",
                GraphCoverage::CompleteSupportedSubset,
                GraphCoverage::CompleteSupportedSubset,
            );
            authority.compilation_id = format!(":empty/{index}");
            authority.generation_id = digest(&format!("empty-generation-id-{index}"));
            authority.generation_ref = object(
                "codeclew-generation-manifest/2.0",
                &format!("empty-generation-{index}"),
            );
            exact_input
                .selected_compilations
                .push(CallableSelectedCompilation {
                    member: member("left"),
                    compilation: authority,
                });
        }
        let exact = build(request.clone(), exact_input.clone()).unwrap();
        assert_eq!(
            exact
                .authority
                .members
                .iter()
                .map(|member| member.compilations.len())
                .sum::<usize>(),
            MAX_CALLABLE_COMPILATIONS
        );
        assert!(exact.authority.members.iter().any(|member| {
            member
                .compilations
                .iter()
                .any(|compilation| compilation.compilation_id == ":empty/63")
        }));

        let mut without_empty = exact_input.clone();
        without_empty.selected_compilations.pop();
        let fewer = build(request.clone(), without_empty).unwrap();
        assert_ne!(
            exact.authority.binding_digest,
            fewer.authority.binding_digest
        );
        assert_ne!(
            exact.authority.authority_digest,
            fewer.authority.authority_digest
        );

        let mut disappeared = exact.clone();
        let authority_member = disappeared
            .authority
            .members
            .iter_mut()
            .find(|member| member.member_alias == "left")
            .unwrap();
        authority_member.compilations.pop();
        let evidence_member = disappeared
            .evidence
            .members
            .iter_mut()
            .find(|member| member.member_alias == "left")
            .unwrap();
        evidence_member.compilations.pop();
        resign(&mut disappeared);
        assert_eq!(
            verify_prepared(&disappeared).unwrap_err().code,
            ErrorCode::StateCorrupt
        );

        let mut over_limit = exact_input;
        let mut extra = compilation(
            "right",
            GraphCoverage::CompleteSupportedSubset,
            GraphCoverage::CompleteSupportedSubset,
        );
        extra.compilation_id = ":empty/64".into();
        extra.generation_id = digest("empty-generation-id-64");
        extra.generation_ref = object("codeclew-generation-manifest/2.0", "empty-generation-64");
        over_limit
            .selected_compilations
            .push(CallableSelectedCompilation {
                member: member("right"),
                compilation: extra,
            });
        assert_eq!(
            build(request, over_limit).unwrap_err().code,
            ErrorCode::SliceBudgetExceeded
        );
    }

    #[test]
    fn top_level_package_owned_function_accepts_empty_containment() {
        let (request, mut build_input) = fixture("p/Orders.findOrder", "p/Consumer.call");
        let mut top_level = descriptor("p/Top.find", "()Ljava/lang/String;", "src/Left.kt", 0);
        top_level["symbolIdentity"] = json!("callable:p/find#jvm:()Ljava/lang/String;");
        top_level["compilerCallableId"] = json!("p/find");
        top_level["ownerIdentity"] = json!("package:p");
        top_level["containment"] = json!([]);
        build_input.payloads[0] = qualified(
            member("left"),
            compilation(
                "left",
                GraphCoverage::CompleteSupportedSubset,
                GraphCoverage::CompleteSupportedSubset,
            ),
            top_level,
        );
        build_input = input(build_input.payloads);
        let prepared = build(request, build_input).unwrap();
        let top_level = facts(&prepared)
            .into_iter()
            .find_map(|fact| match fact {
                CallableFact::Declaration(row)
                    if row.symbol_identity == "callable:p/find#jvm:()Ljava/lang/String;" =>
                {
                    Some(row)
                }
                _ => None,
            })
            .unwrap();
        assert!(top_level.containment.is_empty());
        assert_eq!(top_level.owner_identity, "package:p");
        assert!(top_level.exact_eligible);
    }

    #[test]
    fn task_binding_preserves_a_full_symbol_term() {
        let (mut request, input) = fixture("p/Orders.findOrder", "p/Consumer.call");
        let symbol = "callable:p/Orders.findOrder#jvm:()Ljava/lang/String;";
        request.tasks[0].terms = vec![symbol.into()];
        let prepared = build(request, input).unwrap();
        assert_eq!(prepared.authority.tasks[0].terms, vec![symbol.to_owned()]);
        assert!(
            !String::from_utf8(prepared.projection_bytes)
                .unwrap()
                .contains(symbol)
        );

        for unsafe_term in [
            "callable:p/Api.read#jvm:bad",
            concat!("class:/", "Users/alice/private"),
            "property:p/../secret",
        ] {
            let (mut request, input) = fixture("p/Orders.findOrder", "p/Consumer.call");
            request.tasks[0].terms = vec![unsafe_term.into()];
            assert_eq!(
                build(request, input).unwrap_err().code,
                ErrorCode::InvalidInput
            );
        }
    }

    #[test]
    fn full_symbol_is_exact_but_family_and_token_are_navigation_only() {
        let (request, input) = fixture("p/Orders.findOrder", "p/Orders.findOrder");
        let prepared = build(request, input).unwrap();
        let namespace = prepared
            .authority
            .members
            .iter()
            .find(|member| member.member_alias == "left")
            .unwrap()
            .repository_namespace
            .clone();
        let symbol = "callable:p/Orders.findOrder#jvm:()Ljava/lang/String;";

        let exact = query_prepared(
            &prepared,
            CallableQueryRequest {
                lookups: vec![CallableLookup::FullSymbol {
                    repository_namespace: namespace.clone(),
                    symbol_identity: symbol.into(),
                }],
                max_results: 16,
            },
        )
        .unwrap();
        assert_eq!(exact.status, CallableQueryStatus::ExactFullSymbol);
        assert!(
            exact
                .hits
                .iter()
                .all(|hit| hit.authority == LookupAuthority::ExactFullSymbol)
        );

        let qualified_family = query_prepared(
            &prepared,
            CallableQueryRequest {
                lookups: vec![CallableLookup::CallableFamily {
                    repository_namespace: Some(namespace),
                    callable_id: "p/Orders.findOrder".into(),
                }],
                max_results: 16,
            },
        )
        .unwrap();
        assert_eq!(qualified_family.status, CallableQueryStatus::NavigationOnly);
        let global_family = query_prepared(
            &prepared,
            CallableQueryRequest {
                lookups: vec![CallableLookup::CallableFamily {
                    repository_namespace: None,
                    callable_id: "p/Orders.findOrder".into(),
                }],
                max_results: 16,
            },
        )
        .unwrap();
        assert_eq!(global_family.status, CallableQueryStatus::Ambiguous);

        let token = query_prepared(
            &prepared,
            CallableQueryRequest {
                lookups: vec![CallableLookup::Token {
                    term: "FindOrder".into(),
                }],
                max_results: 16,
            },
        )
        .unwrap();
        assert_eq!(token.status, CallableQueryStatus::NavigationOnly);
        assert!(
            token
                .hits
                .iter()
                .all(|hit| hit.authority != LookupAuthority::ExactFullSymbol)
        );
    }

    #[test]
    fn unrelated_relation_partiality_does_not_downgrade_an_exact_descriptor() {
        let (request, mut build_input) = fixture("p/Orders.findOrder", "p/Consumer.call");
        build_input.payloads[1].compilation.relation_coverage = GraphCoverage::Partial;
        let right_member = build_input.payloads[1].member.clone();
        let right_compilation = build_input.payloads[1].compilation.clone();
        build_input.payloads.push(qualified(
            right_member,
            right_compilation,
            relation_boundary("p/Consumer.unresolved", "src/Right.kt", 20),
        ));
        let prepared = build(request, input(build_input.payloads)).unwrap();
        assert_eq!(
            prepared.authority.completeness.coverage,
            CallableFactSetCoverage::Partial
        );
        let namespace = prepared
            .authority
            .members
            .iter()
            .find(|member| member.member_alias == "left")
            .unwrap()
            .repository_namespace
            .clone();
        let result = query_prepared(
            &prepared,
            CallableQueryRequest {
                lookups: vec![CallableLookup::FullSymbol {
                    repository_namespace: namespace,
                    symbol_identity: "callable:p/Orders.findOrder#jvm:()Ljava/lang/String;".into(),
                }],
                max_results: 16,
            },
        )
        .unwrap();
        assert_eq!(result.status, CallableQueryStatus::ExactFullSymbol);
        assert!(
            result
                .hits
                .iter()
                .all(|hit| hit.authority == LookupAuthority::ExactFullSymbol)
        );
    }

    #[test]
    fn unrelated_relation_boundary_does_not_downgrade_a_proven_use() {
        let left = member("left");
        let right = member("right");
        let left_compilation = compilation(
            "left",
            GraphCoverage::CompleteSupportedSubset,
            GraphCoverage::Partial,
        );
        let right_compilation = compilation(
            "right",
            GraphCoverage::CompleteSupportedSubset,
            GraphCoverage::CompleteSupportedSubset,
        );
        let prepared = build(
            request(RelationshipAuthority::DeclaredTopology),
            input(vec![
                qualified(
                    left.clone(),
                    left_compilation.clone(),
                    descriptor("p/Orders.save", "()V", "src/Left.kt", 0),
                ),
                qualified(
                    left.clone(),
                    left_compilation.clone(),
                    descriptor("p/Orders.write", "()V", "src/Left.kt", 20),
                ),
                qualified(
                    left.clone(),
                    left_compilation.clone(),
                    relation(
                        "p/Orders.save",
                        "callable:p/Orders.write#jvm:()V",
                        "src/Left.kt",
                        10,
                    ),
                ),
                qualified(
                    left,
                    left_compilation,
                    relation_boundary("p/Orders.unrelated", "src/Left.kt", 40),
                ),
                qualified(
                    right,
                    right_compilation,
                    descriptor("p/Consumer.read", "()V", "src/Right.kt", 0),
                ),
            ]),
        )
        .unwrap();
        let use_row = facts(&prepared)
            .into_iter()
            .find_map(|fact| match fact {
                CallableFact::Use(row) if row.source_owner == "p/Orders.save" => Some(row),
                _ => None,
            })
            .unwrap();
        assert!(use_row.exact_eligible);
        assert_eq!(use_row.target_resolution, TargetResolution::ExactSymbol);
        assert!(use_row.uncertainty_reasons.is_empty());
    }

    #[test]
    fn unique_compiler_callable_target_resolves_to_its_full_symbol() {
        let left = member("left");
        let left_compilation = compilation(
            "left",
            GraphCoverage::CompleteSupportedSubset,
            GraphCoverage::CompleteSupportedSubset,
        );
        let mut use_payload = relation(
            "p/Orders.save",
            "callable:p/Orders.write#jvm:()V",
            "src/Left.kt",
            10,
        );
        use_payload["target"] = json!("p/Orders.write");
        let prepared = build(
            request(RelationshipAuthority::DeclaredTopology),
            input(vec![
                qualified(
                    left.clone(),
                    left_compilation.clone(),
                    descriptor("p/Orders.save", "()V", "src/Left.kt", 0),
                ),
                qualified(
                    left.clone(),
                    left_compilation.clone(),
                    descriptor("p/Orders.write", "()V", "src/Left.kt", 20),
                ),
                qualified(left, left_compilation, use_payload),
                qualified(
                    member("right"),
                    compilation(
                        "right",
                        GraphCoverage::CompleteSupportedSubset,
                        GraphCoverage::CompleteSupportedSubset,
                    ),
                    descriptor("p/Consumer.read", "()V", "src/Right.kt", 0),
                ),
            ]),
        )
        .unwrap();
        let use_row = facts(&prepared)
            .into_iter()
            .find_map(|fact| match fact {
                CallableFact::Use(row) if row.source_owner == "p/Orders.save" => Some(row),
                _ => None,
            })
            .unwrap();
        assert_eq!(use_row.target_resolution, TargetResolution::ExactSymbol);
        assert_eq!(
            use_row.target_symbol_identity.as_deref(),
            Some("callable:p/Orders.write#jvm:()V")
        );
        assert!(use_row.exact_eligible);
    }

    #[test]
    fn unrelated_descriptor_boundary_does_not_downgrade_a_proven_shape() {
        let (request, mut build_input) = fixture("p/Orders.findOrder", "p/Consumer.call");
        build_input.payloads[0].compilation.descriptor_coverage = GraphCoverage::Partial;
        let left_member = build_input.payloads[0].member.clone();
        let left_compilation = build_input.payloads[0].compilation.clone();
        build_input.payloads.push(qualified(
            left_member,
            left_compilation,
            boundary(
                "callable:p/Orders.unresolved#jvm:unresolved()V",
                "src/Left.kt",
                20,
            ),
        ));
        let prepared = build(request, input(build_input.payloads)).unwrap();
        let namespace = prepared
            .authority
            .members
            .iter()
            .find(|member| member.member_alias == "left")
            .unwrap()
            .repository_namespace
            .clone();
        let result = query_prepared(
            &prepared,
            CallableQueryRequest {
                lookups: vec![CallableLookup::FullSymbol {
                    repository_namespace: namespace,
                    symbol_identity: "callable:p/Orders.findOrder#jvm:()Ljava/lang/String;".into(),
                }],
                max_results: 16,
            },
        )
        .unwrap();
        assert_eq!(result.status, CallableQueryStatus::ExactFullSymbol);
    }

    #[test]
    fn unidentified_descriptor_row_does_not_downgrade_a_proven_full_symbol() {
        let (request, mut build_input) = fixture("p/Orders.findOrder", "p/Consumer.call");
        build_input.payloads[0].compilation.descriptor_coverage = GraphCoverage::Partial;
        let left_member = build_input.payloads[0].member.clone();
        let left_compilation = build_input.payloads[0].compilation.clone();
        build_input.payloads.push(qualified(
            left_member,
            left_compilation,
            subjectless_descriptor_boundary("src/Left.kt", 20),
        ));
        let prepared = build(request, input(build_input.payloads)).unwrap();
        assert_eq!(
            prepared.authority.completeness.coverage,
            CallableFactSetCoverage::Partial
        );
        let namespace = prepared
            .authority
            .members
            .iter()
            .find(|member| member.member_alias == "left")
            .unwrap()
            .repository_namespace
            .clone();
        let result = query_prepared(
            &prepared,
            CallableQueryRequest {
                lookups: vec![CallableLookup::FullSymbol {
                    repository_namespace: namespace,
                    symbol_identity: "callable:p/Orders.findOrder#jvm:()Ljava/lang/String;".into(),
                }],
                max_results: 16,
            },
        )
        .unwrap();
        assert_eq!(result.status, CallableQueryStatus::ExactFullSymbol);
    }

    #[test]
    fn unidentified_descriptor_row_blocks_only_affected_repository_absence() {
        let (request, mut build_input) = fixture("p/Orders.findOrder", "p/Consumer.call");
        build_input.payloads[0].compilation.descriptor_coverage = GraphCoverage::Partial;
        let left_member = build_input.payloads[0].member.clone();
        let left_compilation = build_input.payloads[0].compilation.clone();
        build_input.payloads.push(qualified(
            left_member,
            left_compilation,
            subjectless_descriptor_boundary("src/Left.kt", 20),
        ));
        let prepared = build(request, input(build_input.payloads)).unwrap();
        let namespaces = prepared
            .authority
            .members
            .iter()
            .map(|member| {
                (
                    member.member_alias.as_str(),
                    member.repository_namespace.clone(),
                )
            })
            .collect::<BTreeMap<_, _>>();

        let affected = query_prepared(
            &prepared,
            CallableQueryRequest {
                lookups: vec![CallableLookup::FullSymbol {
                    repository_namespace: namespaces["left"].clone(),
                    symbol_identity: "callable:p/Orders.missing#jvm:()V".into(),
                }],
                max_results: 16,
            },
        )
        .unwrap();
        assert_eq!(affected.status, CallableQueryStatus::Unsure);
        assert_eq!(
            affected.verification_obligations,
            vec!["VERIFY_PARTIAL_OR_BOUNDARY_EVIDENCE".to_owned()]
        );

        let unaffected = query_prepared(
            &prepared,
            CallableQueryRequest {
                lookups: vec![CallableLookup::FullSymbol {
                    repository_namespace: namespaces["right"].clone(),
                    symbol_identity: "callable:p/Consumer.missing#jvm:()V".into(),
                }],
                max_results: 16,
            },
        )
        .unwrap();
        assert_eq!(unaffected.status, CallableQueryStatus::NotFound);
        assert!(unaffected.verification_obligations.is_empty());
    }

    #[test]
    fn relation_partiality_does_not_block_descriptor_absence() {
        let (request, mut build_input) = fixture("p/Orders.findOrder", "p/Consumer.call");
        build_input.payloads[0].compilation.relation_coverage = GraphCoverage::Partial;
        let prepared = build(request, input(build_input.payloads)).unwrap();
        let namespace = prepared
            .authority
            .members
            .iter()
            .find(|member| member.member_alias == "left")
            .unwrap()
            .repository_namespace
            .clone();
        let result = query_prepared(
            &prepared,
            CallableQueryRequest {
                lookups: vec![CallableLookup::FullSymbol {
                    repository_namespace: namespace,
                    symbol_identity: "callable:p/Orders.missing#jvm:()V".into(),
                }],
                max_results: 16,
            },
        )
        .unwrap();
        assert_eq!(result.status, CallableQueryStatus::NotFound);
        assert!(result.verification_obligations.is_empty());
    }

    #[test]
    fn declared_topology_cannot_upgrade_a_full_relation_target() {
        let (request, mut build_input) = fixture("p/Orders.findOrder", "p/Consumer.call");
        let right = member("right");
        let right_compilation = compilation(
            "right",
            GraphCoverage::CompleteSupportedSubset,
            GraphCoverage::CompleteSupportedSubset,
        );
        build_input.payloads.push(qualified(
            right,
            right_compilation,
            relation(
                "p/Consumer.call",
                "callable:p/Orders.findOrder#jvm:()Ljava/lang/String;",
                "src/Right.kt",
                30,
            ),
        ));
        build_input = input(build_input.payloads);
        let prepared = build(request, build_input).unwrap();
        let use_row = facts(&prepared)
            .into_iter()
            .find_map(|fact| match fact {
                CallableFact::Use(row) => Some(row),
                _ => None,
            })
            .unwrap();
        assert_eq!(use_row.target_resolution, TargetResolution::CallableFamily);
        assert_eq!(
            use_row.relationship_authority,
            RelationshipAuthority::DeclaredTopology
        );
        assert!(!use_row.exact_eligible);
        assert!(use_row.target_symbol_identity.is_none());
    }

    #[test]
    fn sealed_same_snapshot_dependency_can_resolve_one_full_target() {
        let (mut request, mut build_input) = fixture("p/Orders.findOrder", "p/Consumer.call");
        request.pairs[0].relationship_authority =
            RelationshipAuthority::VerifiedSameSnapshotCompilationDependency;
        request.pairs[0].dependency_evidence_ref = Some(object(
            "codeclew-compilation-dependency-evidence/1.0",
            "dependency",
        ));
        build_input.payloads.push(qualified(
            member("right"),
            compilation(
                "right",
                GraphCoverage::CompleteSupportedSubset,
                GraphCoverage::CompleteSupportedSubset,
            ),
            relation(
                "p/Consumer.call",
                "callable:p/Orders.findOrder#jvm:()Ljava/lang/String;",
                "src/Right.kt",
                30,
            ),
        ));
        build_input = input(build_input.payloads);
        let prepared = build(request, build_input).unwrap();
        let use_row = facts(&prepared)
            .into_iter()
            .find_map(|fact| match fact {
                CallableFact::Use(row) => Some(row),
                _ => None,
            })
            .unwrap();
        assert_eq!(use_row.target_resolution, TargetResolution::ExactSymbol);
        assert!(use_row.exact_eligible);
        assert!(use_row.target_repository_namespace.is_some());
    }

    #[test]
    fn relevant_descriptor_partiality_and_boundary_make_full_lookup_unsure() {
        let (request, mut build_input) = fixture("p/Orders.findOrder", "p/Consumer.call");
        let partial = compilation(
            "left",
            GraphCoverage::Partial,
            GraphCoverage::CompleteSupportedSubset,
        );
        build_input.payloads[0] = qualified(
            member("left"),
            partial.clone(),
            descriptor(
                "p/Orders.findOrder",
                "()Ljava/lang/String;",
                "src/Left.kt",
                0,
            ),
        );
        build_input.payloads.push(qualified(
            member("left"),
            partial,
            boundary(
                "callable:p/Orders.findOrder#jvm:()Ljava/lang/String;",
                "src/Left.kt",
                40,
            ),
        ));
        build_input = input(build_input.payloads);
        let prepared = build(request, build_input).unwrap();
        assert_eq!(
            prepared.authority.completeness,
            CallableCompleteness {
                coverage: CallableFactSetCoverage::Partial,
                certainty: CallableFactSetCertainty::Unsure,
                obligation_count: 1,
            }
        );
        let left = prepared
            .authority
            .members
            .iter()
            .find(|member| member.member_alias == "left")
            .unwrap();
        let result = query_prepared(
            &prepared,
            CallableQueryRequest {
                lookups: vec![CallableLookup::FullSymbol {
                    repository_namespace: left.repository_namespace.clone(),
                    symbol_identity: "callable:p/Orders.findOrder#jvm:()Ljava/lang/String;".into(),
                }],
                max_results: 16,
            },
        )
        .unwrap();
        assert_eq!(result.status, CallableQueryStatus::Unsure);
        assert_eq!(result.hits[0].authority, LookupAuthority::Unsure);
    }

    #[test]
    fn truncation_is_a_deterministic_unsure_prefix_with_obligation() {
        let (request, input) = fixture("p/Orders.findOrder", "p/Orders.findOrder");
        let prepared = build(request, input).unwrap();
        let query = CallableQueryRequest {
            lookups: vec![CallableLookup::CallableFamily {
                repository_namespace: None,
                callable_id: "p/Orders.findOrder".into(),
            }],
            max_results: 1,
        };
        let first = query_prepared(&prepared, query.clone()).unwrap();
        let second = query_prepared(&prepared, query).unwrap();
        assert_eq!(first, second);
        assert!(first.truncated);
        assert_eq!(first.status, CallableQueryStatus::Unsure);
        assert_eq!(first.hits.len(), 1);
        assert_eq!(
            first.verification_obligations,
            vec![
                "DISAMBIGUATE_OVERLOAD_SET".to_owned(),
                "NARROW_OR_EXPAND_QUERY".to_owned()
            ]
        );
    }

    #[test]
    fn tampered_payload_duplicate_identity_and_projection_substitution_fail_closed() {
        let (request, input) = fixture("p/Orders.findOrder", "p/Consumer.call");
        let mut tampered = input.clone();
        tampered.payloads[0].payload["returnType"] = json!("kotlin/Int");
        assert_eq!(
            build(request.clone(), tampered).unwrap_err().code,
            ErrorCode::InvalidInput
        );

        let mut duplicate = input.clone();
        duplicate.payloads.push(qualified(
            member("left"),
            compilation(
                "left",
                GraphCoverage::CompleteSupportedSubset,
                GraphCoverage::CompleteSupportedSubset,
            ),
            descriptor(
                "p/Orders.findOrder",
                "()Ljava/lang/String;",
                "src/Other.kt",
                50,
            ),
        ));
        duplicate = super::tests::input(duplicate.payloads);
        assert_eq!(
            build(request.clone(), duplicate).unwrap_err().code,
            ErrorCode::InvalidInput
        );

        let mut prepared = build(request, input).unwrap();
        prepared.projection.authority_digest = digest("substitution");
        assert_eq!(
            verify_prepared(&prepared).unwrap_err().code,
            ErrorCode::StateCorrupt
        );

        let (request, input) = fixture("p/Orders.findOrder", "p/Consumer.call");
        let mut prepared = build(request, input).unwrap();
        prepared.projection.tasks[0].term_count += 1;
        prepared.projection_bytes = canonical::bytes(&prepared.projection).unwrap();
        assert_eq!(
            verify_prepared(&prepared).unwrap_err().code,
            ErrorCode::StateCorrupt
        );
    }

    #[test]
    fn rehashed_evidence_and_query_posting_substitutions_fail_closed() {
        let (request, input) = fixture("p/Orders.findOrder", "p/Consumer.call");
        let original = build(request, input).unwrap();

        let mut evidence_attack = original.clone();
        evidence_attack.evidence.counts.declarations += 1;
        resign(&mut evidence_attack);
        assert_eq!(
            verify_prepared(&evidence_attack).unwrap_err().code,
            ErrorCode::StateCorrupt
        );

        let mut index_attack = original;
        let mut shard: CallableQueryShard =
            serde_json::from_slice(&index_attack.query_shards[0].bytes).unwrap();
        shard.records[0].posting.exact_eligible = !shard.records[0].posting.exact_eligible;
        let bytes = canonical::bytes(&shard).unwrap();
        let reference = CasObject::for_bytes(CALLABLE_QUERY_SHARD_SCHEMA, &bytes).unwrap();
        index_attack.query_shards[0] = PreparedCasObject {
            reference: reference.clone(),
            bytes,
        };
        index_attack.query_index.shards[0].object = reference;
        resign(&mut index_attack);
        assert_eq!(
            verify_prepared(&index_attack).unwrap_err().code,
            ErrorCode::StateCorrupt
        );
    }

    #[test]
    fn resigned_direct_closure_omitting_a_payload_ref_fails_closed() {
        let (request, input) = fixture("p/Orders.findOrder", "p/Consumer.call");
        let mut prepared = build(request, input).unwrap();
        let omitted = facts(&prepared)[0].provenance().input_payload_ref.clone();
        prepared
            .authority
            .direct_cas_closure
            .retain(|reference| reference != &omitted);
        resign(&mut prepared);

        assert!(!prepared.authority.direct_cas_closure.contains(&omitted));
        assert_eq!(
            verify_prepared(&prepared).unwrap_err().code,
            ErrorCode::StateCorrupt
        );
    }

    #[test]
    fn frozen_count_limits_accept_exact_and_reject_limit_plus_one() {
        let budgets = CallableBudgets::frozen();
        assert!(validate_fact_counts(&budgets, MAX_DECLARATION_FACTS, 0, 0).is_ok());
        assert!(validate_fact_counts(&budgets, 1, MAX_USE_FACTS, 0).is_ok());
        assert!(validate_fact_counts(&budgets, 1, 0, MAX_BOUNDARY_FACTS).is_ok());
        assert!(validate_fact_counts(&budgets, MAX_DECLARATION_FACTS, MAX_USE_FACTS, 0).is_ok());
        for counts in [
            (MAX_DECLARATION_FACTS + 1, 0, 0),
            (1, MAX_USE_FACTS + 1, 0),
            (1, 0, MAX_BOUNDARY_FACTS + 1),
            (MAX_DECLARATION_FACTS, MAX_USE_FACTS, 1),
        ] {
            assert_eq!(
                validate_fact_counts(&budgets, counts.0, counts.1, counts.2)
                    .unwrap_err()
                    .code,
                ErrorCode::SliceBudgetExceeded
            );
        }
    }

    #[test]
    fn frozen_request_input_and_query_limits_accept_exact_and_reject_plus_one() {
        assert_exact_limit("pair bindings", MAX_CALLABLE_PAIR_BINDINGS, |count| {
            let mut candidate = request(RelationshipAuthority::DeclaredTopology);
            candidate.pairs = (0..count)
                .map(|index| CallablePairBinding {
                    pair_id: format!("pair-{index}"),
                    provider_member: "left".into(),
                    consumer_member: "right".into(),
                    relationship_authority: RelationshipAuthority::DeclaredTopology,
                    dependency_evidence_ref: None,
                })
                .collect();
            candidate.tasks[0].pair_id = "pair-0".into();
            normalize_request(&mut candidate)
        });

        let (base_request, base_input) = fixture("p/Orders.findOrder", "p/Consumer.call");
        assert_exact_limit("input facts", MAX_INPUT_FACTS_VISITED, |count| {
            let mut candidate = base_input.clone();
            candidate.visited_fact_count = count;
            build(base_request.clone(), candidate).map(|_| ())
        });
        assert_exact_limit("input payload bytes", MAX_INPUT_PAYLOAD_BYTES, |count| {
            let mut candidate = base_input.clone();
            candidate.visited_payload_bytes = count;
            build(base_request.clone(), candidate).map(|_| ())
        });

        let prepared = build(base_request, base_input).unwrap();
        assert_exact_limit("query terms", MAX_CALLABLE_QUERY_TERMS, |count| {
            query_prepared(
                &prepared,
                CallableQueryRequest {
                    lookups: (0..count)
                        .map(|index| CallableLookup::Token {
                            term: format!("term{index}"),
                        })
                        .collect(),
                    max_results: 1,
                },
            )
            .map(|_| ())
        });
        assert_exact_limit("query results", MAX_CALLABLE_QUERY_RESULTS, |count| {
            query_prepared(
                &prepared,
                CallableQueryRequest {
                    lookups: vec![CallableLookup::Token {
                        term: "missing".into(),
                    }],
                    max_results: count,
                },
            )
            .map(|_| ())
        });
    }

    #[test]
    fn frozen_shape_text_and_storage_limits_accept_exact_and_reject_plus_one() {
        let budgets = CallableBudgets::frozen();
        assert_eq!(MAX_CALLABLE_EVIDENCE_OBJECT_BYTES, 64 * 1024 * 1024);
        assert_eq!(MAX_SELECTED_SOURCE_BYTES, 64 * 1024 * 1024);
        assert_eq!(MAX_DIRECT_CAS_CLOSURE_BYTES, 96 * 1024 * 1024);
        assert_exact_limit(
            "parameters per callable",
            MAX_PARAMETERS_PER_CALLABLE,
            |count| {
                validate_declaration_limits(
                    &json!({"parameterTypes": vec![Value::String("kotlin/String".into()); count]}),
                    &budgets,
                )
            },
        );
        assert_exact_limit("type parameters", MAX_TYPE_PARAMETERS, |count| {
            validate_declaration_limits(
                &json!({"typeParameters": vec![json!({"bounds": []}); count]}),
                &budgets,
            )
        });
        assert_exact_limit(
            "bounds per type parameter",
            MAX_BOUNDS_PER_TYPE_PARAMETER,
            |count| {
                validate_declaration_limits(
                    &json!({"typeParameters": [{"bounds": vec![Value::String("kotlin/Any".into()); count]}]}),
                    &budgets,
                )
            },
        );
        assert_exact_limit("containment depth", MAX_CONTAINMENT_DEPTH, |count| {
            string_array(
                &json!({"containment": vec![Value::String("class:p/C".into()); count]}),
                "containment",
                budgets.max_containment_depth,
            )
            .map(|_| ())
        });

        assert_exact_limit("identifier text", MAX_CALLABLE_TEXT_BYTES, |count| {
            required_bounded_string(
                &json!({"identity": "a".repeat(count)}),
                "identity",
                &budgets,
            )
            .map(|_| ())
        });
        assert_exact_limit("type text", MAX_CALLABLE_TEXT_BYTES, |count| {
            validate_json_strings(
                &json!({"returnType": "T".repeat(count)}),
                budgets.max_text_bytes,
                0,
            )
        });
        assert_exact_limit("path text", MAX_CALLABLE_TEXT_BYTES, |count| {
            validate_relative_path(&"p".repeat(count))
        });

        let fact_id = digest("shard-limit");
        let fixed = encoded_fact_shard_len(0, &fact_id, &fact_id, 0, 1).unwrap();
        let encoded_rows_at_limit = budgets.max_shard_bytes - fixed;
        for (encoded_rows, accepted) in [
            (encoded_rows_at_limit, true),
            (encoded_rows_at_limit + 1, false),
        ] {
            let encoded = encoded_fact_shard_len(0, &fact_id, &fact_id, encoded_rows, 1).unwrap();
            assert_eq!(encoded <= budgets.max_shard_bytes, accepted);
        }

        for (fact_shards, query_shards, accepted) in [(31usize, 31usize, true), (31, 32, false)] {
            let derived_objects = fact_shards + query_shards + 2;
            assert_eq!(derived_objects <= budgets.max_derived_cas_objects, accepted);
        }
        assert_exact_limit(
            "retained closure bytes",
            MAX_DIRECT_CAS_CLOSURE_BYTES,
            |count| {
                let mut reference = object("codeclew-test-retained/1.0", "retained");
                reference.size = u64::try_from(count).unwrap();
                canonical_cas_closure(vec![reference], &budgets).map(|_| ())
            },
        );
        for (projection_bytes, accepted) in [
            (MAX_CALLABLE_STDOUT_BYTES - 1, true),
            (MAX_CALLABLE_STDOUT_BYTES, false),
        ] {
            assert_eq!(
                projection_bytes.saturating_add(1) <= budgets.max_stdout_bytes,
                accepted
            );
        }
    }

    #[test]
    fn direct_closure_accounting_is_deduplicated_bounded_and_actionable() {
        let budgets = CallableBudgets::frozen();
        let mut duplicate = object("codeclew-test-retained/1.0", "duplicate");
        duplicate.size = 41;
        let closure =
            canonical_cas_closure(vec![duplicate.clone(), duplicate.clone()], &budgets).unwrap();
        assert_eq!(closure, vec![duplicate.clone()]);

        let mut conflicting = duplicate.clone();
        conflicting.size += 1;
        let conflict = canonical_cas_closure(vec![duplicate, conflicting], &budgets).unwrap_err();
        assert_eq!(conflict.code, ErrorCode::InvalidInput);
        assert!(conflict.message.contains("conflicting metadata"));

        let measured = [
            ("codeclew-generation-manifest/2.0", 2_043u64),
            ("codeclew-kotlin-callable-fact-set-evidence/1.0", 132_930),
            ("codeclew-kotlin-callable-fact-shard/1.0", 10_427_357),
            ("codeclew-kotlin-callable-query-index/1.0", 3_520),
            ("codeclew-kotlin-callable-query-shard/1.0", 54_351_953),
            ("codeclew-kotlin-semantic-fact/3.0", 2_825_511),
            ("codeclew-repository-input-blob/2.0", 98_512),
            ("codeclew-repository-input-snapshot/2.0", 78_904),
        ];
        assert_eq!(
            measured.iter().map(|(_, size)| size).sum::<u64>(),
            67_920_730
        );
        let references = measured
            .iter()
            .map(|(schema, size)| {
                let mut reference = object(schema, schema);
                reference.size = *size;
                reference
            })
            .collect::<Vec<_>>();
        let mut legacy_budget = budgets.clone();
        legacy_budget.max_direct_cas_closure_bytes = 64 * 1024 * 1024;
        let measured_error = canonical_cas_closure(references, &legacy_budget).unwrap_err();
        assert_eq!(measured_error.code, ErrorCode::SliceBudgetExceeded);
        assert!(measured_error.message.contains("uses 67920730 bytes"));
        for (schema, size) in measured {
            assert!(
                measured_error
                    .message
                    .contains(&format!("\"{schema}\": {size}"))
            );
        }

        let mut first = object("codeclew-test-retained/1.0", "overflow-first");
        first.size = u64::MAX;
        let mut second = object("codeclew-test-retained/1.0", "overflow-second");
        second.size = 1;
        let overflow = canonical_cas_closure(vec![first, second], &budgets).unwrap_err();
        assert_eq!(overflow.code, ErrorCode::SliceBudgetExceeded);
        assert!(overflow.message.contains("overflowed"));
    }

    #[test]
    fn callable_authority_rejects_the_previous_closure_budget() {
        let (request, input) = fixture("p/Orders.findOrder", "p/Consumer.call");
        let mut prepared = build(request, input).unwrap();
        prepared.authority.budgets.max_direct_cas_closure_bytes = 64 * 1024 * 1024;
        let error = verify_prepared(&prepared).unwrap_err();
        assert_eq!(error.code, ErrorCode::InvalidInput);
        assert!(
            error
                .message
                .contains("budgets differ from the frozen profile")
        );
    }

    #[test]
    fn incremental_shard_accounting_matches_bytes_and_serializes_only_final_shards() {
        let provenance = CallableFactProvenance {
            member_alias: "left".into(),
            repository_namespace: repository_namespace("repository-left").unwrap(),
            session_id: "session:left".into(),
            session_authority_digest: digest("session-left"),
            base_revision: digest("revision-left"),
            compilation_id: ":app/main".into(),
            generation_id: digest("generation-left"),
            generation_ref: object("codeclew-generation-manifest/2.0", "generation-left"),
            input_fact_key: "kotlin:descriptor:test".into(),
            input_payload_ref: object(KOTLIN_SEMANTIC_FACT_SCHEMA, "payload"),
            source: None,
        };
        let mut rows = (0..512)
            .map(|index| {
                CallableFact::Boundary(BoundaryFact {
                    schema: CALLABLE_FACT_SCHEMA.into(),
                    fact_id: digest(&format!("fact-{index}")),
                    provenance: provenance.clone(),
                    stage: "DECLARATION".into(),
                    code: format!("BOUNDARY_{index}"),
                    subject: Some(format!("subject_{index}")),
                    required_checks: vec![format!("VERIFY_{index}")],
                    boundary_evidence: json!({"ordinal":index}),
                })
            })
            .collect::<Vec<_>>();
        rows.sort_by(|left, right| left.fact_id().cmp(right.fact_id()));
        FACT_SHARD_SERIALIZATIONS.with(|count| count.set(0));
        let (fact_objects, _) = build_fact_shards(&rows, &CallableBudgets::frozen()).unwrap();
        let fact_serializations = FACT_SHARD_SERIALIZATIONS.with(std::cell::Cell::get);
        assert_eq!(fact_serializations, fact_objects.len());
        assert!(fact_serializations < rows.len());
        for object in &fact_objects {
            let shard: CallableFactShard = serde_json::from_slice(&object.bytes).unwrap();
            let row_bytes = shard
                .facts
                .iter()
                .map(|fact| canonical::bytes(fact).unwrap().len())
                .sum();
            assert_eq!(
                encoded_fact_shard_len(
                    shard.sequence,
                    &shard.first_fact_id,
                    &shard.last_fact_id,
                    row_bytes,
                    shard.facts.len(),
                )
                .unwrap(),
                object.bytes.len()
            );
        }

        let records = build_index_records(&rows).unwrap();
        QUERY_SHARD_SERIALIZATIONS.with(|count| count.set(0));
        let binding = digest("binding");
        let (query_objects, _) =
            build_query_shards(&binding, &records, &CallableBudgets::frozen()).unwrap();
        let query_serializations = QUERY_SHARD_SERIALIZATIONS.with(std::cell::Cell::get);
        assert_eq!(query_serializations, query_objects.len());
        assert!(query_serializations < records.len());
        for object in &query_objects {
            let shard: CallableQueryShard = serde_json::from_slice(&object.bytes).unwrap();
            let row_bytes = shard
                .records
                .iter()
                .map(|record| canonical::bytes(record).unwrap().len())
                .sum();
            assert_eq!(
                encoded_query_shard_len(
                    &binding,
                    shard.sequence,
                    canonical::bytes(&shard.first_key).unwrap().len(),
                    canonical::bytes(&shard.last_key).unwrap().len(),
                    row_bytes,
                    shard.records.len(),
                )
                .unwrap(),
                object.bytes.len()
            );
        }
    }
}
