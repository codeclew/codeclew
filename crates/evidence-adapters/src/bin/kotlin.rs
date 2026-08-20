use anyhow::{Context, Result};
use clap::{Parser, ValueEnum};
use clew::error::{ClewError, ErrorCode};
use clew::proto::RequestKind;
use clew::worker::{RequestProfile, WorkerClient, workspace_root};
use evidence_adapters::{
    ADAPTER_OUTPUT_SCHEMA, AdapterIdentity, AdapterOutput, BOUNDARY_EFFECT_ATTRIBUTE,
    BOUNDARY_EFFECT_BUILD_FIDELITY, BOUNDARY_EFFECT_OUT_OF_SCOPE, BOUNDARY_EFFECT_TOPOLOGY,
    CostRecord, IMPACT_QUERY_ENTITY_SCOPE, RepositoryExclusions, SnapshotInput, SourceInput,
    apply_query_boundary_to_frontier, bounded_reverse_impact, canonical_bytes, canonical_hash,
    executable_digest, git_dirty, git_revision, hash_bytes, impact_fact_is_in_scope,
    impact_query_specification, normalize_repo, query_endpoint_key, query_keys_for_entity,
    repo_owned_git_exclusions, required_string, snapshot_repository,
};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::io::Write;
use std::panic::AssertUnwindSafe;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Output};
use std::time::Instant;
use walkdir::WalkDir;

mod kotlin_k1;

use kotlin_k1::{
    AgentGraphLookup, AttemptTelemetry, CacheKey, CacheLookup, KotlinAttempt, PreparedRefusal,
    SemanticCache, read_prepared_refusal, retain_attempt, validate_attempt_destination,
};

const BUILD_STATE_SEED_FILE: &str = "CODECLEW_K1_BUILD_STATE_SEED";
const BUILD_STATE_MANIFEST_FILE: &str = "CODECLEW_K1_BUILD_STATE_MANIFEST.json";
const BUILD_STATE_MANIFEST_SCHEMA: &str = "codeclew.kotlin-k1-build-state-manifest/0.1";

#[derive(Parser)]
#[command(name = "codeclew-kotlin-evidence")]
struct Args {
    #[arg(long)]
    repo: PathBuf,
    #[arg(long, default_value = ":/main")]
    compilation: String,
    #[arg(long)]
    seed_entity: Option<String>,
    #[arg(long, default_value_t = 2)]
    max_depth: usize,
    #[arg(long, default_value_t = 128)]
    max_entities: usize,
    /// Emit a bounded agent-facing impact projection instead of the complete
    /// semantic graph. The full verified graph remains in the cache.
    #[arg(long)]
    agent_output: bool,
    /// Existing external directory used for K1 content-addressed semantic
    /// cache objects. Omitting it disables cross-process reuse.
    #[arg(long)]
    state_root: Option<PathBuf>,
    /// Existing external root for isolated Gradle/Maven/HOME state. When it
    /// is omitted, direct development mode uses repository-local offline
    /// state and reports that weaker authority as an explicit boundary.
    #[arg(long)]
    build_state_root: Option<PathBuf>,
    /// Create-only canonical K1 attempt packet. It must live outside the
    /// source checkout and is never overwritten.
    #[arg(long)]
    attempt_output: Option<PathBuf>,
    /// Execution phase. COLD may answer an explicit agent query and optionally
    /// publish reusable facts; WARM requires an existing verified cache.
    #[arg(long, value_enum, default_value_t = RunPhase::Cold)]
    run_phase: RunPhase,
    /// Canonical per-entry terminal issued by dependency PREPARE. When set,
    /// the adapter validates the sealed authority and never starts a worker.
    #[arg(long)]
    prepared_refusal: Option<PathBuf>,
    #[arg(long)]
    prepared_refusal_sha256: Option<String>,
    #[arg(long)]
    entry_id: Option<String>,
    #[arg(long)]
    candidate_tools_sha256: Option<String>,
    #[arg(long)]
    build_input_digest: Option<String>,
    #[arg(long)]
    preparation_receipt_digest: Option<String>,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum RunPhase {
    Cold,
    Warm,
}

impl RunPhase {
    fn as_str(self) -> &'static str {
        match self {
            Self::Cold => "COLD",
            Self::Warm => "WARM",
        }
    }
}

fn permits_relaxed_agent_graph_lookup(args: &Args) -> bool {
    matches!(args.run_phase, RunPhase::Cold)
        && args.agent_output
        && args.attempt_output.is_none()
        && args.seed_entity.is_some()
}

fn permits_source_syntax_fallback(args: &Args) -> bool {
    matches!(args.run_phase, RunPhase::Cold) && args.agent_output && args.attempt_output.is_none()
}

fn source_syntax_fallback_error(error: &ClewError) -> bool {
    matches!(
        error.code,
        ErrorCode::UnsupportedKotlinVersion
            | ErrorCode::UnsupportedCompilerPluginAbi
            | ErrorCode::UnsupportedProjectConfiguration
            | ErrorCode::WorkerPreparationRequired
            | ErrorCode::IncompleteSemanticAnalysis
    )
}

struct RunSuccess {
    output: Option<AdapterOutput>,
    agent_projection: Option<Value>,
    core: Option<evidence_adapters::CoreBindingSummary>,
    agent_output: bool,
}

struct RunContext {
    total_start: Instant,
    stage: &'static str,
    status: &'static str,
    reason_code: &'static str,
    selected_inputs: Value,
    snapshot: Value,
    provenance: Value,
    boundaries: Vec<Value>,
    cache: Value,
    telemetry: AttemptTelemetry,
    repository: Option<PathBuf>,
    attempt_output: Option<PathBuf>,
    detail_digest_override: Option<String>,
}

#[derive(Default)]
struct BoundaryScopeAggregate {
    row_hashes: BTreeSet<String>,
    affected_row_count: u64,
    owner_query_keys: BTreeSet<String>,
    target_query_keys: BTreeSet<String>,
    has_global_target: bool,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct SourceOccurrenceKey {
    owner: String,
    file: String,
    start: u64,
    end: u64,
}

#[derive(Clone, Debug)]
struct RetainedRelationWitness {
    kind: String,
    operation: String,
    raw_hash: String,
    target_query_key: String,
    resolved_owner: String,
    resolved_target: String,
    target_declaration_kind: Option<String>,
    range: Value,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ExactCallTopologyWitness {
    kind: String,
    operation: String,
    target: String,
    target_query_key: String,
}

#[derive(Default)]
struct ExactCallTopologyOwnerIndex {
    retained_relations: BTreeMap<SourceOccurrenceKey, Vec<ExactCallTopologyWitness>>,
    quarantine_boundaries: BTreeMap<SourceOccurrenceKey, Vec<ExactCallTopologyWitness>>,
    poisoned_quarantine_occurrences: BTreeSet<SourceOccurrenceKey>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct PartialDescriptorCoreWitness {
    source_row_hash: String,
    symbol_identity: String,
    file: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct PartialRelationCoreWitness {
    source_row_hash: String,
    owner: String,
    target: String,
    kind: String,
    file: String,
    start: u64,
    end: u64,
}

#[derive(Default)]
struct PartialCoreIndex {
    descriptors: BTreeMap<String, PartialDescriptorCoreWitness>,
    relations: BTreeMap<String, PartialRelationCoreWitness>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PartialCorePairing {
    NotApplicable,
    Proven,
    Failed,
}

#[derive(Debug)]
struct CompilerBoundaryRelationProof {
    source_boundary_valid: bool,
    owner_query_key: Option<String>,
    target_scope: BoundaryTargetScope,
    target_query_key: Option<String>,
    scoped_operation: Option<String>,
    exact_topology_owner_proven: bool,
    retained_base_operations: BTreeSet<String>,
    derived_fact: Option<Value>,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum BoundaryTargetScope {
    Exact,
    OutOfScope,
    Global,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct RelationEndpointCoverage {
    owner_missing: bool,
    target_missing: bool,
}

impl RelationEndpointCoverage {
    fn new(raw_target: &str, owner_resolved: bool, target_resolved: bool) -> Self {
        let target_needs_identity = raw_target != "null" && !raw_target.starts_with('<');
        Self {
            owner_missing: !owner_resolved,
            target_missing: target_needs_identity && !target_resolved,
        }
    }

    fn endpoints_resolved(self) -> bool {
        !self.owner_missing && !self.target_missing
    }

    fn role_missing(self, role: &str) -> bool {
        match role {
            "OWNER" => self.owner_missing,
            "TARGET" => self.target_missing,
            _ => false,
        }
    }
}

fn strict_sha256_digest(value: Option<&str>) -> Option<&str> {
    let value = value?;
    (value.len() == 71
        && value.starts_with("sha256:")
        && value[7..]
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)))
    .then_some(value)
}

fn optional_descriptor_attribute_boundary(code: &str) -> bool {
    matches!(
        code,
        "UNKNOWN_VISIBILITY"
            | "UNKNOWN_EFFECTIVE_VISIBILITY"
            | "UNKNOWN_MODALITY"
            | "UNRESOLVED_DESCRIPTOR_TYPE"
    )
}

impl PartialCoreIndex {
    /// Index only the provider row that survived entity materialization. A
    /// boundary cannot prove retained topology merely by naming a row that
    /// was present in the provider graph before adapter filtering/deduping.
    fn index_emitted_descriptor(&mut self, descriptor: &Value) -> Result<()> {
        if descriptor.get("attributeCoverage").and_then(Value::as_str) != Some("PARTIAL") {
            return Ok(());
        }
        let Some(source_row_hash) =
            strict_sha256_digest(descriptor.get("sourceRowHash").and_then(Value::as_str))
        else {
            return Ok(());
        };
        if descriptor.get("schema").and_then(Value::as_str) != Some("declaration-descriptor/0.1")
            || descriptor.get("resolution").and_then(Value::as_str) != Some("PROVEN")
            || descriptor.get("provider").and_then(Value::as_str) != Some("K2_FIR")
        {
            return Ok(());
        }
        let (Some(symbol_identity), Some(file)) = (
            descriptor.get("symbolIdentity").and_then(Value::as_str),
            descriptor.get("file").and_then(Value::as_str),
        ) else {
            return Ok(());
        };
        if symbol_identity.is_empty() || file.is_empty() {
            return Ok(());
        }
        self.descriptors.insert(
            canonical_hash(descriptor)?,
            PartialDescriptorCoreWitness {
                source_row_hash: source_row_hash.to_owned(),
                symbol_identity: symbol_identity.to_owned(),
                file: file.to_owned(),
            },
        );
        Ok(())
    }

    /// Index the strict-valid raw partial core independently of endpoint
    /// materialization. If an endpoint is unavailable, the separately
    /// emitted unresolved-endpoint boundary owns that topology gap while the
    /// paired type boundary remains only an optional-attribute limitation.
    fn index_verified_relation_core(&mut self, relation: &Value) -> Result<()> {
        if relation.get("attributeCoverage").and_then(Value::as_str) != Some("PARTIAL") {
            return Ok(());
        }
        let Some(source_row_hash) =
            strict_sha256_digest(relation.get("sourceRowHash").and_then(Value::as_str))
        else {
            return Ok(());
        };
        if relation.get("schema").and_then(Value::as_str) != Some("declaration-relation/0.1")
            || relation.get("resolution").and_then(Value::as_str) != Some("PROVEN")
            || relation.get("provider").and_then(Value::as_str) != Some("K2_FIR")
        {
            return Ok(());
        }
        let (Some(owner), Some(target), Some(kind), Some(file), Some(start), Some(end)) = (
            relation.get("owner").and_then(Value::as_str),
            relation.get("target").and_then(Value::as_str),
            relation.get("kind").and_then(Value::as_str),
            relation.get("file").and_then(Value::as_str),
            relation.get("start").and_then(Value::as_u64),
            relation.get("end").and_then(Value::as_u64),
        ) else {
            return Ok(());
        };
        if !matches!(kind, "CALLS" | "CONSTRUCTS")
            || owner.is_empty()
            || target.is_empty()
            || file.is_empty()
            || file.starts_with('<')
            || Path::new(file).is_absolute()
            || Path::new(file)
                .components()
                .any(|component| matches!(component, Component::ParentDir))
            || end < start
        {
            return Ok(());
        }
        self.relations.insert(
            canonical_hash(relation)?,
            PartialRelationCoreWitness {
                source_row_hash: source_row_hash.to_owned(),
                owner: owner.to_owned(),
                target: target.to_owned(),
                kind: kind.to_owned(),
                file: file.to_owned(),
                start,
                end,
            },
        );
        Ok(())
    }

    fn boundary_pairing(
        &self,
        domain: &str,
        provider: &str,
        stage: &str,
        code: &str,
        boundary: &Value,
    ) -> PartialCorePairing {
        let eligible_descriptor = domain == "descriptor"
            && provider == "COMPILER_DESCRIPTOR_NORMALIZER"
            && stage == "NORMALIZE"
            && optional_descriptor_attribute_boundary(code);
        let relation_candidate = domain == "relation"
            && provider == "COMPILER_RELATION_NORMALIZER"
            && stage == "NORMALIZE"
            && code == "UNRESOLVED_RELATION_TYPE";
        if !eligible_descriptor && !relation_candidate {
            return PartialCorePairing::NotApplicable;
        }
        let expected_schema = if eligible_descriptor {
            "declaration-descriptor-boundary/0.1"
        } else {
            "declaration-relation-boundary/0.1"
        };
        if boundary.get("schema").and_then(Value::as_str) != Some(expected_schema)
            || boundary.get("resolution").and_then(Value::as_str) != Some("UNKNOWN")
            || boundary.get("provider").and_then(Value::as_str) != Some(provider)
            || boundary.get("stage").and_then(Value::as_str) != Some(stage)
            || boundary.get("code").and_then(Value::as_str) != Some(code)
        {
            return PartialCorePairing::Failed;
        }
        let raw_row_hash = strict_sha256_digest(boundary.get("rawRowHash").and_then(Value::as_str));
        if raw_row_hash.is_none() {
            return PartialCorePairing::Failed;
        }

        if relation_candidate {
            let relation_kind = boundary.get("relationKind").and_then(Value::as_str);
            let retained_link_claimed = boundary.get("retainedRelationHash").is_some();
            let retained_call_kind = matches!(relation_kind, Some("CALLS" | "CONSTRUCTS"));
            if !retained_call_kind && !retained_link_claimed {
                return if relation_kind
                    .and_then(compiler_relation_operation)
                    .is_some()
                {
                    PartialCorePairing::NotApplicable
                } else {
                    PartialCorePairing::Failed
                };
            }
        }

        if eligible_descriptor {
            let retained_hash = strict_sha256_digest(
                boundary
                    .get("retainedDescriptorHash")
                    .and_then(Value::as_str),
            );
            let witness = retained_hash.and_then(|hash| self.descriptors.get(hash));
            return if witness.is_some_and(|witness| {
                raw_row_hash == Some(witness.source_row_hash.as_str())
                    && boundary.get("symbolIdentity").and_then(Value::as_str)
                        == Some(witness.symbol_identity.as_str())
                    && boundary.get("file").and_then(Value::as_str) == Some(witness.file.as_str())
            }) {
                PartialCorePairing::Proven
            } else {
                PartialCorePairing::Failed
            };
        }

        let retained_hash =
            strict_sha256_digest(boundary.get("retainedRelationHash").and_then(Value::as_str));
        let witness = retained_hash.and_then(|hash| self.relations.get(hash));
        if witness.is_some_and(|witness| {
            raw_row_hash == Some(witness.source_row_hash.as_str())
                && boundary.get("owner").and_then(Value::as_str) == Some(witness.owner.as_str())
                && boundary.get("target").and_then(Value::as_str) == Some(witness.target.as_str())
                && boundary.get("relationKind").and_then(Value::as_str)
                    == Some(witness.kind.as_str())
                && boundary.get("file").and_then(Value::as_str) == Some(witness.file.as_str())
                && boundary.get("start").and_then(Value::as_u64) == Some(witness.start)
                && boundary.get("end").and_then(Value::as_u64) == Some(witness.end)
        }) {
            PartialCorePairing::Proven
        } else {
            PartialCorePairing::Failed
        }
    }
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct BoundaryTargetBucket {
    scope: BoundaryTargetScope,
    target_query_key: Option<String>,
}

fn boundary_target_bucket(
    scope: BoundaryTargetScope,
    target_query_key: Option<String>,
) -> BoundaryTargetBucket {
    match (scope, target_query_key) {
        (BoundaryTargetScope::Exact, Some(target_query_key)) => BoundaryTargetBucket {
            scope,
            target_query_key: Some(target_query_key),
        },
        (BoundaryTargetScope::Exact, None) => BoundaryTargetBucket {
            scope: BoundaryTargetScope::Global,
            target_query_key: None,
        },
        (scope, _) => BoundaryTargetBucket {
            scope,
            target_query_key: None,
        },
    }
}

fn query_key_for_raw_endpoint(identity: &str) -> Result<Option<String>> {
    if identity.is_empty() || identity == "null" || identity.starts_with('<') {
        return Ok(None);
    }
    Ok(Some(query_endpoint_key(identity)?))
}

fn boundary_target_scope(identity: Option<&str>) -> Result<(BoundaryTargetScope, Option<String>)> {
    let query_key = identity
        .map(query_key_for_raw_endpoint)
        .transpose()?
        .flatten();
    let scope = match (identity, query_key.as_ref()) {
        (_, Some(_)) => BoundaryTargetScope::Exact,
        (Some(identity), None)
            if identity.is_empty() || identity == "null" || identity.starts_with('<') =>
        {
            BoundaryTargetScope::OutOfScope
        }
        _ => BoundaryTargetScope::Global,
    };
    Ok((scope, query_key))
}

fn source_occurrence_key(value: &Value) -> Result<Option<SourceOccurrenceKey>> {
    let Some(owner) = value.get("owner").and_then(Value::as_str) else {
        return Ok(None);
    };
    if owner.is_empty() {
        return Ok(None);
    }
    let Some(file) = value.get("file").and_then(Value::as_str) else {
        return Ok(None);
    };
    if file.is_empty()
        || file.starts_with('<')
        || Path::new(file).is_absolute()
        || Path::new(file)
            .components()
            .any(|component| matches!(component, Component::ParentDir))
    {
        return Ok(None);
    }
    let Some(start) = value.get("start").and_then(Value::as_u64) else {
        return Ok(None);
    };
    let Some(end) = value.get("end").and_then(Value::as_u64) else {
        return Ok(None);
    };
    if end < start {
        return Ok(None);
    }
    Ok(Some(SourceOccurrenceKey {
        owner: owner.to_owned(),
        file: file.to_owned(),
        start,
        end,
    }))
}

fn argument_mapping_boundary_code(code: &str) -> bool {
    matches!(
        code,
        "ARGUMENT_OWNER_NOT_FUNCTION"
            | "NO_COMPILER_CALLABLE_ID"
            | "EXTERNAL_OR_LOCAL_ARGUMENT_TARGET"
            | "EXTENSION_ARGUMENT_MAPPING_UNSUPPORTED"
            | "CONTEXT_ARGUMENT_MAPPING_UNSUPPORTED"
            | "VARARG_ARGUMENT_MAPPING_UNSUPPORTED"
            | "MISSING_RESOLVED_ARGUMENT_MAPPING"
            | "UNRESOLVED_PARAMETER_IDENTITY"
            | "INCOMPLETE_ARGUMENT_MAPPING"
    )
}

fn raw_base_argument_mapping_boundary_code(code: &str) -> bool {
    matches!(
        code,
        "EXTENSION_ARGUMENT_MAPPING_UNSUPPORTED"
            | "CONTEXT_ARGUMENT_MAPPING_UNSUPPORTED"
            | "VARARG_ARGUMENT_MAPPING_UNSUPPORTED"
            | "MISSING_RESOLVED_ARGUMENT_MAPPING"
            | "INCOMPLETE_ARGUMENT_MAPPING"
    )
}

fn compiler_relation_operation(kind: &str) -> Option<String> {
    matches!(
        kind,
        "OVERRIDES"
            | "CALLS"
            | "REFERENCES"
            | "CONSTRUCTS"
            | "READS"
            | "WRITES"
            | "INITIALIZES"
            | "NULL_COALESCES"
            | "RETURNS_VALUE_FROM"
    )
    .then(|| format!("codeclew.relation/{}/1", kind.to_ascii_lowercase()))
}

impl ExactCallTopologyOwnerIndex {
    /// Retain the exact compiler row even when adapter endpoint resolution
    /// cannot materialize it as an agent fact. The separately emitted
    /// unresolved-endpoint boundary remains responsible for that topology
    /// gap; this witness proves only that argument mapping is optional to the
    /// already identified CALLS/CONSTRUCTS occurrence.
    fn index_verified_relation(&mut self, relation: &Value) -> Result<()> {
        if relation.get("schema").and_then(Value::as_str) != Some("declaration-relation/0.1")
            || relation.get("resolution").and_then(Value::as_str) != Some("PROVEN")
            || relation.get("provider").and_then(Value::as_str) != Some("K2_FIR")
        {
            return Ok(());
        }
        let Some(kind @ ("CALLS" | "CONSTRUCTS")) = relation.get("kind").and_then(Value::as_str)
        else {
            return Ok(());
        };
        let (Some(owner), Some(target), Some(file)) = (
            relation.get("owner").and_then(Value::as_str),
            relation.get("target").and_then(Value::as_str),
            relation.get("file").and_then(Value::as_str),
        ) else {
            return Ok(());
        };
        if owner.is_empty()
            || target.is_empty()
            || file.is_empty()
            || Path::new(file).is_absolute()
            || Path::new(file)
                .components()
                .any(|component| matches!(component, Component::ParentDir))
        {
            return Ok(());
        }
        let Some(occurrence) = source_occurrence_key(relation)? else {
            return Ok(());
        };
        let Some(target_query_key) = query_key_for_raw_endpoint(target)? else {
            return Ok(());
        };
        let operation = compiler_relation_operation(kind).expect("call kind has an operation");
        self.retained_relations
            .entry(occurrence)
            .or_default()
            .push(ExactCallTopologyWitness {
                kind: kind.to_owned(),
                operation,
                target: target.to_owned(),
                target_query_key,
            });
        Ok(())
    }

    /// A quarantined-descriptor boundary owns the missing edge topology. Its
    /// exact source occurrence can therefore prove that a separate argument
    /// mapping boundary is attribute-only without inventing a relation fact.
    fn index_quarantine_boundary(&mut self, boundary: &Value) -> Result<()> {
        let claimed_owner = boundary.get("provider").and_then(Value::as_str)
            == Some("COMPILER_RELATION_NORMALIZER")
            && boundary.get("stage").and_then(Value::as_str) == Some("NORMALIZE")
            && boundary.get("code").and_then(Value::as_str)
                == Some("REFERENCE_TO_QUARANTINED_DESCRIPTOR");
        if !claimed_owner {
            return Ok(());
        }
        let Some(occurrence) = source_occurrence_key(boundary)? else {
            return Ok(());
        };
        let exact_header = boundary.get("schema").and_then(Value::as_str)
            == Some("declaration-relation-boundary/0.1")
            && boundary.get("resolution").and_then(Value::as_str) == Some("UNKNOWN")
            && strict_sha256_digest(boundary.get("rawRowHash").and_then(Value::as_str)).is_some()
            && boundary.get("retainedRelationHash").is_none();
        let kind = boundary.get("relationKind").and_then(Value::as_str);
        let target = boundary.get("target").and_then(Value::as_str);
        let target_query_key = target
            .map(query_key_for_raw_endpoint)
            .transpose()?
            .flatten();
        if !exact_header
            || !matches!(kind, Some("CALLS" | "CONSTRUCTS"))
            || target_query_key.is_none()
        {
            self.poisoned_quarantine_occurrences.insert(occurrence);
            return Ok(());
        }
        let kind = kind.expect("validated call kind");
        let target = target.expect("validated exact target");
        let target_query_key = target_query_key.expect("validated exact target key");
        let operation = compiler_relation_operation(kind).expect("call kind has an operation");
        self.quarantine_boundaries
            .entry(occurrence)
            .or_default()
            .push(ExactCallTopologyWitness {
                kind: kind.to_owned(),
                operation,
                target: target.to_owned(),
                target_query_key,
            });
        Ok(())
    }

    fn retained_candidates(
        &self,
        occurrence: Option<&SourceOccurrenceKey>,
    ) -> &[ExactCallTopologyWitness] {
        occurrence
            .and_then(|key| self.retained_relations.get(key))
            .map(Vec::as_slice)
            .unwrap_or_default()
    }

    fn quarantine_candidates(
        &self,
        occurrence: Option<&SourceOccurrenceKey>,
    ) -> &[ExactCallTopologyWitness] {
        occurrence
            .and_then(|key| self.quarantine_boundaries.get(key))
            .map(Vec::as_slice)
            .unwrap_or_default()
    }

    fn quarantine_poisoned(&self, occurrence: Option<&SourceOccurrenceKey>) -> bool {
        occurrence.is_some_and(|key| self.poisoned_quarantine_occurrences.contains(key))
    }
}

fn compiler_boundary_relation_proof(
    domain: &str,
    provider: &str,
    stage: &str,
    code: &str,
    boundary: &Value,
    topology_owners: &ExactCallTopologyOwnerIndex,
    retained_relations: &BTreeMap<SourceOccurrenceKey, Vec<RetainedRelationWitness>>,
    repository_tree_digest: &str,
) -> Result<CompilerBoundaryRelationProof> {
    let expected_schema = match domain {
        "relation" => Some("declaration-relation-boundary/0.1"),
        "descriptor" => Some("declaration-descriptor-boundary/0.1"),
        _ => None,
    };
    let source_boundary_valid = expected_schema.is_some()
        && boundary.get("schema").and_then(Value::as_str) == expected_schema
        && boundary.get("resolution").and_then(Value::as_str) == Some("UNKNOWN")
        && boundary.get("provider").and_then(Value::as_str) == Some(provider)
        && boundary.get("stage").and_then(Value::as_str) == Some(stage)
        && boundary.get("code").and_then(Value::as_str) == Some(code)
        && !matches!(provider, "" | "UNKNOWN")
        && !matches!(stage, "" | "UNKNOWN")
        && !matches!(code, "" | "UNKNOWN");
    let owner_query_key = if source_boundary_valid {
        boundary
            .get("owner")
            .and_then(Value::as_str)
            .map(query_key_for_raw_endpoint)
            .transpose()?
            .flatten()
    } else {
        None
    };

    let occurrence = if source_boundary_valid && domain == "relation" {
        source_occurrence_key(boundary)?
    } else {
        None
    };
    let scoped_normalizer_boundary = domain == "relation"
        && provider == "COMPILER_RELATION_NORMALIZER"
        && stage == "NORMALIZE"
        && matches!(
            code,
            "REFERENCE_TO_QUARANTINED_DESCRIPTOR" | "UNRESOLVED_RELATION_TYPE"
        );
    let normalized_relation_operation = scoped_normalizer_boundary
        .then(|| {
            boundary
                .get("relationKind")
                .and_then(Value::as_str)
                .and_then(compiler_relation_operation)
        })
        .flatten();
    let argument_mapping_boundary = source_boundary_valid
        && domain == "relation"
        && provider == "K2_FIR"
        && stage == "ARGUMENT_MAPPING"
        && argument_mapping_boundary_code(code);
    let raw_base_mapping_boundary =
        argument_mapping_boundary && raw_base_argument_mapping_boundary_code(code);
    let retained_owner_candidates = if raw_base_mapping_boundary {
        topology_owners.retained_candidates(occurrence.as_ref())
    } else {
        &[]
    };
    let quarantine_owner_candidates = if raw_base_mapping_boundary {
        topology_owners.quarantine_candidates(occurrence.as_ref())
    } else {
        &[]
    };
    let topology_owner_ambiguous = retained_owner_candidates.len() > 1
        || quarantine_owner_candidates.len() > 1
        || (!retained_owner_candidates.is_empty() && !quarantine_owner_candidates.is_empty());
    let topology_owner_candidate = if topology_owner_ambiguous {
        None
    } else {
        retained_owner_candidates
            .first()
            .or_else(|| quarantine_owner_candidates.first())
    };
    // Current producer mapping boundaries do not repeat the base target/kind.
    // If a future row does, a present disagreement is corruption, not
    // permission to recover from a nearby owner or REFERENCES fallback.
    let topology_owner_metadata_conflict = topology_owner_candidate.is_some_and(|witness| {
        boundary
            .get("target")
            .is_some_and(|value| value.as_str() != Some(witness.target.as_str()))
            || boundary
                .get("relationKind")
                .is_some_and(|value| value.as_str() != Some(witness.kind.as_str()))
    });
    let topology_owner_failed = topology_owner_ambiguous
        || topology_owner_metadata_conflict
        || (raw_base_mapping_boundary && topology_owners.quarantine_poisoned(occurrence.as_ref()));
    let exact_topology_owner = (!topology_owner_failed)
        .then_some(topology_owner_candidate)
        .flatten();
    let topology_owners_absent =
        retained_owner_candidates.is_empty() && quarantine_owner_candidates.is_empty();

    // A present but malformed target is not equivalent to missing metadata:
    // do not recover around it from a nearby source occurrence. Normalizer
    // topology gaps receive exact scope only under their complete core
    // identity contract; otherwise the old fail-closed GLOBAL behavior stays.
    let direct_target = if !source_boundary_valid {
        Some((BoundaryTargetScope::Global, None))
    } else if topology_owner_failed {
        Some((BoundaryTargetScope::Global, None))
    } else if let Some(witness) = exact_topology_owner {
        Some((
            BoundaryTargetScope::Exact,
            Some(witness.target_query_key.clone()),
        ))
    } else if scoped_normalizer_boundary
        && (occurrence.is_none() || normalized_relation_operation.is_none())
    {
        Some((BoundaryTargetScope::Global, None))
    } else {
        match boundary.get("target") {
            None => None,
            Some(Value::String(target)) => Some(boundary_target_scope(Some(target))?),
            Some(_) => Some((BoundaryTargetScope::Global, None)),
        }
    };
    let direct_target = if direct_target.is_none() && domain == "descriptor" {
        match boundary.get("symbolIdentity") {
            None => None,
            Some(Value::String(identity)) => Some(boundary_target_scope(Some(identity))?),
            Some(_) => Some((BoundaryTargetScope::Global, None)),
        }
    } else {
        direct_target
    };
    let witnesses = occurrence
        .as_ref()
        .and_then(|key| retained_relations.get(key))
        .map(Vec::as_slice)
        .unwrap_or_default();

    // Prefer one exact topology owner: either a retained relation row or the
    // quarantine boundary that owns the missing edge. Only a complete absence
    // of both owners permits the legacy unique REFERENCES recovery. Legacy
    // mapping codes intentionally ignore this new owner contract.
    let references_fallback_allowed = argument_mapping_boundary
        && (!raw_base_mapping_boundary || (topology_owners_absent && !topology_owner_failed));
    let source_candidates = if references_fallback_allowed {
        witnesses
            .iter()
            .filter(|witness| witness.kind == "REFERENCES")
            .collect::<Vec<_>>()
    } else if domain == "relation"
        && provider == "K2_FIR_CFG"
        && stage == "ORDER_PROVENANCE"
        && code == "NO_CFG_NODE_FOR_RELATION"
    {
        witnesses
            .iter()
            .filter(|witness| {
                matches!(
                    witness.kind.as_str(),
                    "CALLS"
                        | "CONSTRUCTS"
                        | "READS"
                        | "WRITES"
                        | "NULL_COALESCES"
                        | "RETURNS_VALUE_FROM"
                )
            })
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };

    let unique_source_witness = match source_candidates.as_slice() {
        [witness] => Some(*witness),
        _ => None,
    };
    let (target_scope, target_query_key) = match direct_target {
        Some(target) => target,
        None => unique_source_witness.map_or((BoundaryTargetScope::Global, None), |witness| {
            (
                BoundaryTargetScope::Exact,
                Some(witness.target_query_key.clone()),
            )
        }),
    };

    let expected_operations = compiler_boundary_operations(stage, code)
        .into_iter()
        .collect::<BTreeSet<_>>();
    let mut retained_base_operations = BTreeSet::new();
    if let Some(witness) = exact_topology_owner {
        retained_base_operations.insert(witness.operation.clone());
    } else if let Some(target_key) = target_query_key.as_deref() {
        for witness in witnesses {
            if witness.target_query_key == target_key
                && expected_operations.contains(&witness.operation)
            {
                retained_base_operations.insert(witness.operation.clone());
            }
        }
    }
    if provider == "K2_FIR_CFG"
        && stage == "ORDER_PROVENANCE"
        && code == "NO_CFG_NODE_FOR_RELATION"
        && let Some(witness) = unique_source_witness
    {
        retained_base_operations.insert(witness.operation.clone());
    }

    let mut derived_fact = None;
    if retained_base_operations.is_empty()
        && argument_mapping_boundary
        && !topology_owner_failed
        && target_scope == BoundaryTargetScope::Exact
        && let Some(witness) = unique_source_witness
        && target_query_key.as_deref() == Some(witness.target_query_key.as_str())
    {
        let derived_kind = match witness.target_declaration_kind.as_deref() {
            Some("FUNCTION") => Some("CALLS"),
            Some("CONSTRUCTOR") => Some("CONSTRUCTS"),
            _ => None,
        };
        if let Some(derived_kind) = derived_kind {
            let operation = format!("codeclew.relation/{}/1", derived_kind.to_ascii_lowercase());
            let source_boundary_hash = canonical_hash(boundary)?;
            let fact_id = canonical_hash(&json!({
                "provider":"K2_FIR_DERIVED_CALL_TOPOLOGY",
                "kind":derived_kind,
                "owner":witness.resolved_owner,
                "target":witness.resolved_target,
                "range":witness.range,
                "snapshot":repository_tree_digest,
                "referenceRelationHash":witness.raw_hash,
                "sourceBoundaryHash":source_boundary_hash,
            }))?;
            derived_fact = Some(json!({
                "factId":fact_id,
                "relation":operation,
                "owner":witness.resolved_owner,
                "target":witness.resolved_target,
                "truth":"TRUE",
                "grade":"COMPILER_RESOLVED",
                "enumeration":"PARTIAL",
                "range":witness.range,
                "providerPayload":{
                    "schema":"codeclew.derived-call-topology/0.1",
                    "authority":"K2_ARGUMENT_MAPPING_BOUNDARY_PLUS_UNIQUE_REFERENCE_AND_DESCRIPTOR_KIND",
                    "referenceRelationHash":witness.raw_hash,
                    "sourceBoundaryHash":source_boundary_hash,
                    "targetDeclarationKind":witness.target_declaration_kind,
                },
            }));
            retained_base_operations.insert(operation);
        }
    }

    let scoped_operation = (target_scope != BoundaryTargetScope::Global)
        .then(|| {
            exact_topology_owner
                .map(|witness| witness.operation.clone())
                .or(normalized_relation_operation)
        })
        .flatten();
    Ok(CompilerBoundaryRelationProof {
        source_boundary_valid,
        owner_query_key,
        target_scope,
        target_query_key,
        scoped_operation,
        exact_topology_owner_proven: exact_topology_owner.is_some(),
        retained_base_operations,
        derived_fact,
    })
}

fn boundary_applicability(
    effect: &str,
    operations: impl IntoIterator<Item = String>,
    owner_query_keys: &BTreeSet<String>,
    target_query_keys: &BTreeSet<String>,
    exact_target_scope: bool,
) -> Value {
    let operations = operations.into_iter().collect::<BTreeSet<_>>();
    let mut applicability = json!({
        "schema":"codeclew.boundary-applicability/0.1",
        "effect":effect,
        "operations":operations,
        "direction":"INCOMING",
        "locality":if exact_target_scope {"EXACT"} else {"GLOBAL"},
        "ownerQueryKeys":owner_query_keys,
        "targetQueryKeys":target_query_keys,
    });
    if effect == BOUNDARY_EFFECT_OUT_OF_SCOPE {
        applicability["entityScope"] = Value::String(IMPACT_QUERY_ENTITY_SCOPE.to_owned());
    }
    applicability
}

fn global_boundary_applicability(effect: &str) -> Value {
    boundary_applicability(
        effect,
        std::iter::empty(),
        &BTreeSet::new(),
        &BTreeSet::new(),
        false,
    )
}

fn compiler_boundary_effect(
    domain: &str,
    provider: &str,
    stage: &str,
    code: &str,
    retained_base_relation: bool,
    source_boundary_valid: bool,
) -> &'static str {
    if !source_boundary_valid {
        return BOUNDARY_EFFECT_TOPOLOGY;
    }
    // These two boundaries are emitted only after the base relation has been
    // retained. K2_FIR_CFG adds NO_CFG beside that relation, and the Codeclew
    // normalizer removes only argumentToParameter while retaining CALLS or
    // CONSTRUCTS. They therefore describe optional evidence, not a missing
    // topology edge, even when the aggregate boundary no longer carries a
    // row-level target witness.
    let producer_guarantees_retained_base = (provider == "K2_FIR_CFG"
        && stage == "ORDER_PROVENANCE"
        && code == "NO_CFG_NODE_FOR_RELATION")
        || (provider == "CODECLEW_RELATION_NORMALIZER"
            && stage == "OPTIONAL_RELATION_EVIDENCE"
            && code == "ARGUMENT_MAPPING_UNAVAILABLE");
    let optional_relation_attribute = producer_guarantees_retained_base
        || (domain == "relation"
            && provider == "K2_FIR"
            && stage == "ARGUMENT_MAPPING"
            && argument_mapping_boundary_code(code));
    let excluded_by_entity_scope = (domain == "relation"
        && provider == "K2_FIR"
        && stage == "ARGUMENT_MAPPING"
        && code == "EXTERNAL_OR_LOCAL_ARGUMENT_TARGET")
        || (domain == "descriptor"
            && provider == "K2_FIR"
            && stage == "CONSTRUCTOR_DECLARATION"
            && code == "LOCAL_CONSTRUCTOR_UNSUPPORTED");
    if excluded_by_entity_scope {
        BOUNDARY_EFFECT_OUT_OF_SCOPE
    } else if producer_guarantees_retained_base
        || (optional_relation_attribute && retained_base_relation)
    {
        BOUNDARY_EFFECT_ATTRIBUTE
    } else if domain == "descriptor"
        && stage == "DECLARATION"
        && matches!(
            code,
            "GENERATED_OR_NO_SOURCE" | "LOCAL_DECLARATION_UNSUPPORTED"
        )
    {
        BOUNDARY_EFFECT_OUT_OF_SCOPE
    } else {
        BOUNDARY_EFFECT_TOPOLOGY
    }
}

fn apply_partial_core_pairing(
    proof: &mut CompilerBoundaryRelationProof,
    pairing: PartialCorePairing,
) {
    if pairing == PartialCorePairing::Failed {
        proof.target_scope = BoundaryTargetScope::Global;
        proof.target_query_key = None;
        proof.scoped_operation = None;
        proof.exact_topology_owner_proven = false;
        proof.retained_base_operations.clear();
        proof.derived_fact = None;
    }
}

fn compiler_boundary_effect_with_partial_core(
    domain: &str,
    provider: &str,
    stage: &str,
    code: &str,
    retained_base_relation: bool,
    source_boundary_valid: bool,
    target_scope: BoundaryTargetScope,
    pairing: PartialCorePairing,
) -> &'static str {
    let non_call_type_out_of_scope = source_boundary_valid
        && domain == "relation"
        && provider == "COMPILER_RELATION_NORMALIZER"
        && stage == "NORMALIZE"
        && code == "UNRESOLVED_RELATION_TYPE"
        && target_scope == BoundaryTargetScope::OutOfScope;
    match pairing {
        PartialCorePairing::Proven if source_boundary_valid => BOUNDARY_EFFECT_ATTRIBUTE,
        PartialCorePairing::Proven => BOUNDARY_EFFECT_TOPOLOGY,
        PartialCorePairing::Failed => BOUNDARY_EFFECT_TOPOLOGY,
        PartialCorePairing::NotApplicable if non_call_type_out_of_scope => {
            BOUNDARY_EFFECT_OUT_OF_SCOPE
        }
        PartialCorePairing::NotApplicable => compiler_boundary_effect(
            domain,
            provider,
            stage,
            code,
            retained_base_relation,
            source_boundary_valid,
        ),
    }
}

fn compiler_boundary_operations(stage: &str, code: &str) -> Vec<String> {
    let kinds: &[&str] = match (stage, code) {
        ("REFERENCE", _) => &["references"],
        ("OVERRIDE", _) => &["overrides"],
        ("INITIALIZER", _) => &["initializes"],
        ("WRITE", _) => &["writes"],
        ("RETURN_VALUE", _) => &["returns_value_from"],
        ("ARGUMENT_MAPPING", _) | (_, "ARGUMENT_MAPPING_UNAVAILABLE") => &["calls", "constructs"],
        ("NULL_POLICY", _) => &["null_coalesces"],
        (_, "NULL_COALESCING_FLOW_UNAVAILABLE") => &["null_coalesces"],
        (_, "RETURN_VALUE_FLOW_UNAVAILABLE") => &["returns_value_from"],
        _ => &[],
    };
    kinds
        .iter()
        .map(|kind| format!("codeclew.relation/{kind}/1"))
        .collect()
}

fn compiler_boundary_operations_for_proof(
    stage: &str,
    code: &str,
    proof: &CompilerBoundaryRelationProof,
) -> BTreeSet<String> {
    let mut operations = if proof.exact_topology_owner_proven {
        BTreeSet::new()
    } else {
        compiler_boundary_operations(stage, code)
            .into_iter()
            .collect()
    };
    operations.extend(proof.scoped_operation.iter().cloned());
    operations
}

impl RunContext {
    fn new() -> Self {
        Self {
            total_start: Instant::now(),
            stage: "ARGUMENT_PARSING",
            status: "FAILED",
            reason_code: "INVALID_ARGUMENTS",
            selected_inputs: json!({}),
            snapshot: json!({}),
            provenance: json!({"adapterId":"codeclew.kotlin-k2","adapterVersion":"0.1.0"}),
            boundaries: Vec::new(),
            cache: json!({"status":"NOT_INITIALIZED","hit":false}),
            telemetry: AttemptTelemetry::new(),
            repository: None,
            attempt_output: None,
            detail_digest_override: None,
        }
    }

    fn enter(&mut self, stage: &'static str, status: &'static str, reason_code: &'static str) {
        self.stage = stage;
        self.status = status;
        self.reason_code = reason_code;
    }
}

fn retain_operational_profiles(context: &mut RunContext, profile: &RequestProfile) {
    let Some(cache) = context.cache.as_object_mut() else {
        return;
    };
    if let Some(compiler_index) = profile.compiler_index.as_ref()
        && let Ok(value) = serde_json::to_value(compiler_index)
    {
        cache.insert("compilerIndex".to_owned(), value);
    }
    if let Some(project_model_cache) = profile.project_model_cache.as_ref()
        && let Ok(value) = serde_json::to_value(project_model_cache)
    {
        cache.insert("projectModelCache".to_owned(), value);
    }
}

fn main() {
    let code = supervised_main();
    if code != 0 {
        std::process::exit(code);
    }
}

fn supervised_main() -> i32 {
    let mut context = RunContext::new();
    let args = match Args::try_parse() {
        Ok(args) => args,
        Err(error) => {
            return emit_terminal(&mut context, &error.to_string());
        }
    };
    context.attempt_output = args.attempt_output.clone();
    context.selected_inputs = json!({
        "compilation":args.compilation,
        "runPhase":args.run_phase.as_str(),
        "query":{
            "requestedSeedEntity":args.seed_entity,
            "maxDepth":args.max_depth,
            "maxEntities":args.max_entities,
        },
        "semanticCacheRequested":args.state_root.is_some(),
        "agentOutputRequested":args.agent_output,
        "externalBuildStateRequested":args.build_state_root.is_some(),
        "preparedRefusalRequested":args.prepared_refusal.is_some(),
    });
    let result = std::panic::catch_unwind(AssertUnwindSafe(|| run(args, &mut context)));
    match result {
        Ok(Ok(success)) => emit_success(success, &mut context),
        Ok(Err(error)) => {
            classify_error(&error, &mut context);
            emit_terminal(&mut context, &format!("{error:#}"))
        }
        Err(payload) => {
            context.enter("PANIC", "FAILED", "ADAPTER_PANIC");
            let detail = payload
                .downcast_ref::<&str>()
                .copied()
                .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
                .unwrap_or("non-string panic payload");
            emit_terminal(&mut context, detail)
        }
    }
}

fn emit_success(success: RunSuccess, context: &mut RunContext) -> i32 {
    context.telemetry.external_wall_micros = context.total_start.elapsed().as_micros() as u64;
    if let Some(output) = &success.output {
        context.telemetry.fact_count = output.facts.len() as u64;
        context.telemetry.boundary_count = output.boundaries.len() as u64;
    }
    let serialization_start = Instant::now();
    let output_bytes = match match (&success.output, &success.agent_projection) {
        (Some(output), None) if success.agent_output => {
            agent_impact_projection(output).and_then(|projection| canonical_bytes(&projection))
        }
        (Some(output), None) => canonical_bytes(output),
        (None, Some(projection)) if success.agent_output => canonical_bytes(projection),
        _ => Err(anyhow::anyhow!("success payload is ambiguous or absent")),
    } {
        Ok(bytes) => bytes,
        Err(error) => {
            context.enter(
                "SERIALIZATION",
                "FAILED",
                "ADAPTER_OUTPUT_SERIALIZATION_FAILED",
            );
            return emit_terminal(context, &error.to_string());
        }
    };
    context.telemetry.serialization_micros = serialization_start.elapsed().as_micros() as u64;
    context.telemetry.external_wall_micros = context.total_start.elapsed().as_micros() as u64;
    context.telemetry.emitted_bytes = output_bytes.len() as u64 + 1;
    if let Some(path) = &context.attempt_output {
        let Some(output) = success.output.as_ref() else {
            context.enter("ATTEMPT_SEAL", "FAILED", "ATTEMPT_SEAL_FAILED");
            return emit_terminal(context, "retained success requires full adapter output");
        };
        let Some(core) = success.core else {
            context.enter("ATTEMPT_SEAL", "FAILED", "ATTEMPT_SEAL_FAILED");
            return emit_terminal(
                context,
                "retained success requires full evidence-core validation",
            );
        };
        let attempt = match KotlinAttempt::success(
            output,
            core,
            context.selected_inputs.clone(),
            context.provenance.clone(),
            context.cache.clone(),
            context.telemetry.clone(),
        ) {
            Ok(attempt) => attempt,
            Err(error) => {
                context.enter("ATTEMPT_SEAL", "FAILED", "ATTEMPT_SEAL_FAILED");
                return emit_terminal(context, &error.to_string());
            }
        };
        if let Err(error) = retain_attempt(path, context.repository.as_deref(), &attempt) {
            context.enter("ATTEMPT_RETENTION", "FAILED", "ATTEMPT_RETENTION_FAILED");
            return emit_terminal(context, &error.to_string());
        }
    }
    let mut stdout = std::io::stdout().lock();
    if stdout
        .write_all(&output_bytes)
        .and_then(|_| stdout.write_all(b"\n"))
        .is_err()
    {
        return 2;
    }
    0
}

fn indexed_query_boundaries(
    index: &kotlin_k1::AgentGraphIndex,
    selected_entities: &[Value],
) -> Result<Vec<Value>> {
    let mut frontier = BTreeSet::new();
    for entity in selected_entities {
        frontier.extend(query_keys_for_entity(entity)?);
    }
    if frontier.is_empty() {
        anyhow::bail!("indexed impact query has no canonical entity keys");
    }
    let mut scheduled = frontier.clone();
    let mut pending = VecDeque::from_iter(frontier.iter().cloned());
    let mut assessed = BTreeSet::new();
    let mut relevant = BTreeMap::new();
    while let Some(query_key) = pending.pop_front() {
        for boundary in index.boundaries_for_query_key(&query_key)? {
            let digest = canonical_hash(&boundary)?;
            if !assessed.insert(digest.clone()) {
                continue;
            }
            if apply_query_boundary_to_frontier(&boundary, &mut frontier) {
                relevant.insert(digest, boundary);
            }
            for discovered in &frontier {
                if scheduled.insert(discovered.clone()) {
                    pending.push_back(discovered.clone());
                }
            }
        }
    }
    let mut boundaries = relevant.into_values().collect::<Vec<_>>();
    boundaries.sort_by(|left, right| {
        left.get("boundaryId")
            .and_then(Value::as_str)
            .cmp(&right.get("boundaryId").and_then(Value::as_str))
    });
    Ok(boundaries)
}

fn traversable_boundary_frontier_entities(
    selected_entities: &[Value],
    affected: &[Value],
    max_depth: usize,
) -> Vec<Value> {
    let traversable_entity_ids = affected
        .iter()
        .filter(|row| {
            row.get("depth")
                .and_then(Value::as_u64)
                .is_none_or(|depth| depth < max_depth as u64)
        })
        .filter_map(|row| row.get("entityId").and_then(Value::as_str))
        .collect::<BTreeSet<_>>();
    selected_entities
        .iter()
        .filter(|entity| {
            entity
                .get("opaqueId")
                .and_then(Value::as_str)
                .is_some_and(|entity_id| traversable_entity_ids.contains(entity_id))
        })
        .cloned()
        .collect()
}

#[allow(clippy::too_many_arguments)]
fn indexed_agent_impact_projection(
    lookup: &AgentGraphLookup,
    requested_seed: &str,
    max_depth: usize,
    max_entities: usize,
    repository_tree_digest: &str,
    vcs_revision: Option<&str>,
    dirty: bool,
    compilation: &str,
) -> Result<Value> {
    let started = Instant::now();
    let authority = lookup.index.projection_authority();
    let expected_vcs = vcs_revision.map_or(Value::Null, |value| Value::String(value.to_owned()));
    if authority.get("schema").and_then(Value::as_str)
        != Some("codeclew.kotlin-agent-projection-authority/0.1")
        || authority.pointer("/adapter/id").and_then(Value::as_str) != Some("codeclew.kotlin-k2")
        || authority
            .pointer("/adapter/version")
            .and_then(Value::as_str)
            != Some("0.1.0")
        || authority
            .pointer("/snapshot/repositoryTreeDigest")
            .and_then(Value::as_str)
            != Some(repository_tree_digest)
        || authority.pointer("/snapshot/vcsRevision") != Some(&expected_vcs)
        || authority
            .pointer("/snapshot/dirty")
            .and_then(Value::as_bool)
            != Some(dirty)
        || authority
            .pointer("/snapshot/selectedCompilation")
            .and_then(Value::as_str)
            != Some(compilation)
        || authority
            .pointer("/compilerReceipt/method")
            .and_then(Value::as_str)
            != Some("K2_FIR_ANALYSIS")
        || authority
            .pointer("/compilerReceipt/status")
            .and_then(Value::as_str)
            != Some("ACCEPTED")
        || authority
            .pointer("/compilerReceipt/grade")
            .and_then(Value::as_str)
            != Some("COMPILER_CHECKED")
        || authority
            .pointer("/compilerReceipt/k2Validated")
            .and_then(Value::as_bool)
            != Some(true)
        || authority
            .pointer("/compilerReceipt/snapshotTreeDigest")
            .and_then(Value::as_str)
            != Some(repository_tree_digest)
        || authority.pointer("/compilerReceipt/adapterBinaryDigest")
            != authority.pointer("/adapter/binaryDigest")
        || authority
            .pointer("/snapshot/targets")
            .and_then(Value::as_array)
            .is_none()
    {
        anyhow::bail!("agent graph projection authority differs from the live query");
    }

    let seed = lookup.index.resolve_seed(requested_seed)?;
    let seed_id = seed.entity_id;
    let mut selected_entities = vec![seed.entity];
    let mut selected_entity_ids = BTreeSet::from([seed_id.clone()]);
    let mut affected = vec![json!({
        "entityId":seed_id,
        "impactClass":"DEFINITE",
        "depth":0,
    })];
    let mut queue = VecDeque::from([(seed_id.clone(), 0usize)]);
    let mut paths = Vec::new();
    let mut path_facts = Vec::new();
    while let Some((target, depth)) = queue.pop_front() {
        if depth >= max_depth || affected.len() >= max_entities {
            continue;
        }
        for fact in lookup.index.incoming_facts(&target)? {
            if fact.get("target").and_then(Value::as_str) != Some(target.as_str()) {
                anyhow::bail!("agent graph incoming shard contains a mismatched target");
            }
            if !impact_fact_is_in_scope(&fact) {
                continue;
            }
            let owner = required_string(&fact, "/owner")?.to_owned();
            let fact_id = required_string(&fact, "/factId")?.to_owned();
            paths.push(json!({
                "from":target,
                "to":owner,
                "factId":fact_id,
                "relation":fact.get("relation"),
            }));
            path_facts.push(fact);
            if selected_entity_ids.insert(owner.clone()) {
                let entity = lookup.index.entity(&owner)?.with_context(|| {
                    format!("agent graph fact references missing entity {owner}")
                })?;
                if entity.get("opaqueId").and_then(Value::as_str) != Some(owner.as_str()) {
                    anyhow::bail!("agent graph entity identity differs from the fact endpoint");
                }
                selected_entities.push(entity);
                affected.push(json!({
                    "entityId":owner,
                    "impactClass":"POSSIBLE",
                    "depth":depth + 1,
                }));
                queue.push_back((owner, depth + 1));
            }
            if affected.len() >= max_entities {
                break;
            }
        }
    }
    let path_fact_ids = prove_displayed_paths(&paths, &path_facts, &selected_entity_ids)?;
    let project_summary = lookup.index.project_boundary_summary();
    let project_boundary_count = project_summary
        .get("count")
        .and_then(Value::as_u64)
        .context("agent graph project boundary summary has no count")?;
    let project_boundary_set_digest = required_string(&project_summary, "/setDigest")?;
    let project_global_boundary_count = project_summary
        .get("globalCount")
        .and_then(Value::as_u64)
        .context("agent graph project boundary summary has no global count")?;
    let boundary_frontier_entities =
        traversable_boundary_frontier_entities(&selected_entities, &affected, max_depth);
    let mut query_boundaries =
        indexed_query_boundaries(&lookup.index, &boundary_frontier_entities)?;
    let query_relevant_project_count = query_boundaries.len();
    let mut query_local_boundary_count = 0usize;
    if affected.len() >= max_entities {
        query_boundaries.push(json!({
            "boundaryId":canonical_hash(&json!({
                "kind":"budget-max-entities",
                "maxEntities":max_entities,
            }))?,
            "kindUri":"codeclew.boundary/budget-max-entities/1",
            "consequence":"ENUMERATION_INCOMPLETE",
            "origin":Value::Null,
            "details":{"maxEntities":max_entities},
            "applicability":{
                "effect":BOUNDARY_EFFECT_TOPOLOGY,
                "direction":"INCOMING",
                "locality":"GLOBAL",
                "ownerQueryKeys":[],
                "targetQueryKeys":[],
            },
        }));
        query_local_boundary_count = 1;
    }
    let query_specification = impact_query_specification();
    let query_specification_digest = canonical_hash(&query_specification)?;
    let boundary_assessment = json!({
        "schema":"codeclew.query-boundary-assessment/0.1",
        "policy":"EXPLICIT_APPLICABILITY_FAIL_CLOSED",
        "querySpecificationDigest":query_specification_digest,
        "projectBoundaryCount":project_boundary_count,
        "projectBoundarySetDigest":project_boundary_set_digest,
        "projectGlobalBoundaryCount":project_global_boundary_count,
        "queryRelevantProjectBoundaryCount":query_relevant_project_count,
        "queryLocalBoundaryCount":query_local_boundary_count,
        "queryRelevantBoundaryCount":query_boundaries.len(),
        "queryRelevantBoundarySetDigest":canonical_hash(&query_boundaries)?,
    });
    let complete = query_boundaries.is_empty();
    let mut closure = json!({
        "id":"impact-closure-completeness",
        "kind":"codeclew.obligation/impact-closure-completeness/1",
        "mandatory":true,
        "status":if complete {"SATISFIED"} else {"UNKNOWN"},
        "evidenceFactIds":path_fact_ids,
        "providerPayload":{
            "boundaryAssessmentDigest":canonical_hash(&boundary_assessment)?,
            "querySpecificationDigest":query_specification_digest,
            "queryRelevantBoundaryCount":query_boundaries.len(),
            "queryRelevantBoundarySetDigest":canonical_hash(&query_boundaries)?,
        },
    });
    if !complete {
        closure["reason"] = Value::String("QUERY_TOPOLOGY_BOUNDARY_REMAINS".to_owned());
    }
    let query_micros = started.elapsed().as_micros() as u64;
    let impact = json!({
        "schema":"codeclew.impact-result/0.1",
        "status":if complete {"COMPLETE_IN_SCOPE"} else {"PARTIAL_BOUNDARY"},
        "closureSpecification":evidence_adapters::IMPACT_CLOSURE_SPEC,
        "querySpecification":query_specification,
        "seedEntity":seed_id,
        "maxDepth":max_depth,
        "maxEntities":max_entities,
        "affected":affected,
        "paths":paths,
        "mandatoryObligations":[closure],
        "boundaries":query_boundaries,
        "boundaryAssessment":boundary_assessment,
        "providerPayload":{
            "proposedSeedEntity":seed_id,
            "selectionAuthority":"CALLER_SELECTED_RESOLVED_ENTITY",
        },
        "pathProof":{
            "schema":"codeclew.displayed-path-proof/0.1",
            "status":"SATISFIED",
            "claim":"DISPLAYED_REVERSE_IMPACT_PATHS_ARE_COMPILER_PROVEN",
            "pathCount":paths.len(),
            "pathFactCount":path_facts.len(),
            "pathFactSetDigest":canonical_hash(&json!(path_facts))?,
            "pathFactIds":path_fact_ids,
            "querySpecificationDigest":query_specification_digest,
        },
        "projectWarnings":{
            "schema":"codeclew.project-boundary-inventory/0.1",
            "coverage":"FULL_PROJECT_DIGEST_AND_KIND_SUMMARY",
            "boundaryCount":project_boundary_count,
            "boundarySetDigest":project_boundary_set_digest,
            "globalBoundaryCount":project_global_boundary_count,
            "byKind":project_summary.get("byKind"),
        },
        "queryMicros":query_micros,
    });
    Ok(json!({
        "schema":"codeclew.agent-impact-projection/0.2",
        "adapter":{
            "adapterId":authority.pointer("/adapter/id"),
            "version":authority.pointer("/adapter/version"),
            "binaryDigest":authority.pointer("/adapter/binaryDigest"),
            "languageId":authority.pointer("/adapter/languageId"),
        },
        "snapshot":{
            "repositoryTreeDigest":authority.pointer("/snapshot/repositoryTreeDigest"),
            "vcsRevision":authority.pointer("/snapshot/vcsRevision"),
            "dirty":authority.pointer("/snapshot/dirty"),
            "targets":authority.pointer("/snapshot/targets"),
        },
        "compiler":{
            "method":authority.pointer("/compilerReceipt/method"),
            "status":authority.pointer("/compilerReceipt/status"),
            "grade":authority.pointer("/compilerReceipt/grade"),
            "k2Validated":authority.pointer("/compilerReceipt/k2Validated"),
        },
        "projectionAuthority":authority,
        "impact":impact,
        "selectedEntities":selected_entities,
        "pathFacts":path_facts,
        "semanticOutputDigest":authority.get("semanticOutputDigest"),
    }))
}

fn agent_impact_projection(output: &AdapterOutput) -> Result<Value> {
    let entities = output
        .entities
        .iter()
        .filter_map(|entity| {
            entity
                .get("opaqueId")
                .and_then(Value::as_str)
                .map(|id| (id, entity))
        })
        .collect::<BTreeMap<_, _>>();
    let facts = output
        .facts
        .iter()
        .filter_map(|fact| {
            fact.get("factId")
                .and_then(Value::as_str)
                .map(|id| (id, fact))
        })
        .collect::<BTreeMap<_, _>>();
    let affected = output
        .impact
        .get("affected")
        .and_then(Value::as_array)
        .context("impact result has no affected entities")?;
    let selected_entities = affected
        .iter()
        .map(|row| {
            let id = required_string(row, "/entityId")?;
            entities
                .get(id)
                .copied()
                .cloned()
                .with_context(|| format!("impact references missing entity {id}"))
        })
        .collect::<Result<Vec<_>>>()?;
    let paths = output
        .impact
        .get("paths")
        .and_then(Value::as_array)
        .context("impact result has no paths")?;
    let path_facts = paths
        .iter()
        .map(|path| {
            let id = required_string(path, "/factId")?;
            facts
                .get(id)
                .copied()
                .cloned()
                .with_context(|| format!("impact path references missing fact {id}"))
        })
        .collect::<Result<Vec<_>>>()?;
    let selected_entity_ids = selected_entities
        .iter()
        .filter_map(|entity| entity.get("opaqueId").and_then(Value::as_str))
        .map(ToOwned::to_owned)
        .collect::<BTreeSet<_>>();
    let scoped_impact = agent_scoped_impact(
        &output.impact,
        &path_facts,
        &selected_entity_ids,
        &output.compiler_receipt,
        &output.boundaries,
    )?;
    Ok(json!({
        "schema":"codeclew.agent-impact-projection/0.2",
        "adapter":output.adapter,
        "snapshot":{
            "repositoryTreeDigest":output.snapshot_input.repository_tree_digest,
            "vcsRevision":output.snapshot_input.vcs_revision,
            "dirty":output.snapshot_input.dirty,
            "targets":output.snapshot_input.targets,
        },
        "compiler":{
            "method":output.compiler_receipt.get("method"),
            "status":output.compiler_receipt.get("status"),
            "grade":output.compiler_receipt.get("grade"),
            "k2Validated":output.compiler_receipt.pointer("/providerPayload/k2Validated"),
        },
        "impact":scoped_impact,
        "selectedEntities":selected_entities,
        "pathFacts":path_facts,
        "semanticOutputDigest":kotlin_k1::semantic_output_digest(output)?,
    }))
}

fn prove_displayed_paths(
    paths: &[Value],
    path_facts: &[Value],
    selected_entity_ids: &BTreeSet<String>,
) -> Result<BTreeSet<String>> {
    if paths.len() != path_facts.len() {
        anyhow::bail!("agent impact path and fact counts differ");
    }
    let mut path_fact_ids = BTreeSet::new();
    for (path, fact) in paths.iter().zip(path_facts) {
        let fact_id = required_string(fact, "/factId")?;
        if !path_fact_ids.insert(fact_id.to_owned())
            || path.get("factId").and_then(Value::as_str) != Some(fact_id)
            || path.get("from").and_then(Value::as_str)
                != fact.get("target").and_then(Value::as_str)
            || path.get("to").and_then(Value::as_str) != fact.get("owner").and_then(Value::as_str)
            || path.get("relation") != fact.get("relation")
            || fact.get("truth").and_then(Value::as_str) != Some("TRUE")
            || fact.get("grade").and_then(Value::as_str) != Some("COMPILER_RESOLVED")
        {
            anyhow::bail!("agent impact contains a path without an exact compiler-resolved fact");
        }
        for endpoint in ["owner", "target"] {
            let id = fact
                .get(endpoint)
                .and_then(Value::as_str)
                .with_context(|| format!("agent path fact has no {endpoint}"))?;
            if !selected_entity_ids.contains(id) {
                anyhow::bail!("agent path fact endpoint is absent from selected entities");
            }
        }
        let range = fact
            .get("range")
            .and_then(Value::as_object)
            .context("agent path fact has no exact source range")?;
        if range.get("artifactId").and_then(Value::as_str).is_none()
            || range
                .get("artifactContentDigest")
                .and_then(Value::as_str)
                .is_none()
            || range.get("startByte").and_then(Value::as_u64).is_none()
            || range.get("endByte").and_then(Value::as_u64).is_none()
        {
            anyhow::bail!("agent path fact source range is incomplete");
        }
    }
    Ok(path_fact_ids)
}

fn agent_scoped_impact(
    impact: &Value,
    path_facts: &[Value],
    selected_entity_ids: &BTreeSet<String>,
    compiler_receipt: &Value,
    project_boundaries: &[Value],
) -> Result<Value> {
    let query_specification = impact_query_specification();
    let query_specification_digest = canonical_hash(&query_specification)?;
    if impact.get("querySpecification") != Some(&query_specification) {
        anyhow::bail!("agent impact has no exact query specification");
    }
    if compiler_receipt.get("method").and_then(Value::as_str) != Some("K2_FIR_ANALYSIS")
        || compiler_receipt.get("status").and_then(Value::as_str) != Some("ACCEPTED")
        || compiler_receipt.get("grade").and_then(Value::as_str) != Some("COMPILER_CHECKED")
        || compiler_receipt
            .pointer("/providerPayload/k2Validated")
            .and_then(Value::as_bool)
            != Some(true)
    {
        anyhow::bail!("agent impact paths require an accepted K2 compiler receipt");
    }
    let paths = impact
        .get("paths")
        .and_then(Value::as_array)
        .context("impact result has no paths")?;
    let path_fact_ids = prove_displayed_paths(paths, path_facts, selected_entity_ids)?;

    let query_boundaries = impact
        .get("boundaries")
        .and_then(Value::as_array)
        .context("impact result has no boundaries")?;
    let obligations = impact
        .get("mandatoryObligations")
        .and_then(Value::as_array)
        .context("impact result has no mandatory obligations")?;
    let [closure] = obligations.as_slice() else {
        anyhow::bail!("agent impact must expose exactly one closure-completeness obligation");
    };
    if closure.get("kind").and_then(Value::as_str)
        != Some("codeclew.obligation/impact-closure-completeness/1")
        || closure.get("mandatory").and_then(Value::as_bool) != Some(true)
        || closure.get("status").and_then(Value::as_str)
            != Some(if query_boundaries.is_empty() {
                "SATISFIED"
            } else {
                "UNKNOWN"
            })
    {
        anyhow::bail!("agent impact closure obligation contradicts query boundaries");
    }
    let assessment = impact
        .get("boundaryAssessment")
        .context("impact result has no boundary assessment")?;
    let project_boundary_set_digest = canonical_hash(&project_boundaries)?;
    let query_boundary_set_digest = canonical_hash(&query_boundaries)?;
    if assessment
        .get("projectBoundarySetDigest")
        .and_then(Value::as_str)
        != Some(project_boundary_set_digest.as_str())
        || assessment
            .get("queryRelevantBoundarySetDigest")
            .and_then(Value::as_str)
            != Some(query_boundary_set_digest.as_str())
        || assessment
            .get("queryRelevantBoundaryCount")
            .and_then(Value::as_u64)
            != Some(query_boundaries.len() as u64)
        || assessment
            .get("querySpecificationDigest")
            .and_then(Value::as_str)
            != Some(query_specification_digest.as_str())
    {
        anyhow::bail!("agent impact boundary assessment is not inventory-bound");
    }
    let mut by_kind = BTreeMap::<String, (u64, u64)>::new();
    for boundary in project_boundaries {
        let kind = boundary
            .get("kindUri")
            .and_then(Value::as_str)
            .unwrap_or("codeclew.boundary/unknown/1")
            .to_owned();
        let affected_rows = boundary
            .pointer("/details/affectedRowCount")
            .and_then(Value::as_u64)
            .unwrap_or(1);
        let aggregate = by_kind.entry(kind).or_default();
        aggregate.0 = aggregate.0.saturating_add(1);
        aggregate.1 = aggregate.1.saturating_add(affected_rows);
    }
    let kind_summary = by_kind
        .into_iter()
        .map(|(kind_uri, (boundary_count, affected_row_count))| {
            json!({
                "kindUri":kind_uri,
                "boundaryCount":boundary_count,
                "affectedRowCount":affected_row_count,
            })
        })
        .collect::<Vec<_>>();

    let mut scoped = impact.clone();
    scoped["pathProof"] = json!({
        "schema":"codeclew.displayed-path-proof/0.1",
        "status":"SATISFIED",
        "claim":"DISPLAYED_REVERSE_IMPACT_PATHS_ARE_COMPILER_PROVEN",
        "pathCount":paths.len(),
        "pathFactCount":path_facts.len(),
        "pathFactSetDigest":canonical_hash(&json!(path_facts))?,
        "pathFactIds":path_fact_ids,
        "querySpecificationDigest":query_specification_digest,
    });
    scoped["projectWarnings"] = json!({
        "schema":"codeclew.project-boundary-inventory/0.1",
        "coverage":"FULL_PROJECT_DIGEST_AND_KIND_SUMMARY",
        "boundaryCount":project_boundaries.len(),
        "boundarySetDigest":canonical_hash(&project_boundaries)?,
        "byKind":kind_summary,
    });
    Ok(scoped)
}

fn classify_error(error: &anyhow::Error, context: &mut RunContext) {
    let Some(error) = error.downcast_ref::<clew::error::ClewError>() else {
        return;
    };
    if std::env::var_os("CODECLEW_K1_LOCAL_DIAGNOSTICS").as_deref()
        == Some(std::ffi::OsStr::new("1"))
    {
        for evidence in &error.evidence {
            eprintln!("codeclew diagnostic: {evidence}");
        }
    }
    let reason = serde_json::to_value(&error.code)
        .ok()
        .and_then(|value| value.as_str().map(ToOwned::to_owned));
    let Some(reason) = reason else {
        return;
    };
    context.reason_code = match reason.as_str() {
        "UNSUPPORTED_KOTLIN_VERSION" => "UNSUPPORTED_KOTLIN_VERSION",
        "UNSUPPORTED_COMPILER_PLUGIN_ABI" => "COMPILER_PLUGIN_ABI_MISMATCH",
        "UNSUPPORTED_PROJECT_CONFIGURATION" => "UNSUPPORTED_PROJECT_CONFIGURATION",
        "PROJECT_MODEL_CHANGED" => "PROJECT_MODEL_CHANGED",
        "WORKER_PROTOCOL_MISMATCH" => "WORKER_PROTOCOL_MISMATCH",
        "WORKER_PREPARATION_REQUIRED" => "WORKER_PREPARATION_REQUIRED",
        "WORKER_CRASHED" => "WORKER_CRASHED",
        "INCOMPLETE_SEMANTIC_ANALYSIS" => "INCOMPLETE_SEMANTIC_ANALYSIS",
        "INVALID_INPUT" => "INVALID_INPUT",
        _ => context.reason_code,
    };
    context.status = match error.code {
        clew::error::ErrorCode::UnsupportedKotlinVersion
        | clew::error::ErrorCode::UnsupportedProjectConfiguration
        | clew::error::ErrorCode::WorkerPreparationRequired => "REFUSED",
        clew::error::ErrorCode::IncompleteSemanticAnalysis
        | clew::error::ErrorCode::SliceBudgetExceeded
        | clew::error::ErrorCode::UnsupportedCompilerPluginAbi => "PARTIAL",
        _ => "FAILED",
    };
    if let Some(details) = compiler_plugin_abi_diagnostic(error, context) {
        context.reason_code = "COMPILER_PLUGIN_ABI_MISMATCH";
        context.status = "PARTIAL";
        let boundary = json!({
            "boundaryId":canonical_hash(&json!({"kind":"compiler-plugin-abi-mismatch","details":details}))
                .unwrap_or_else(|_| hash_bytes(b"compiler-plugin-abi-mismatch")),
            "kindUri":"codeclew.boundary/kotlin/compiler-plugin-abi-mismatch/1",
            "consequence":"SEMANTIC_ANALYSIS_UNAVAILABLE",
            "origin":Value::Null,
            "provider":"KOTLIN_COMPILER",
            "applicability":global_boundary_applicability(BOUNDARY_EFFECT_TOPOLOGY),
            "details":details,
        });
        context.boundaries.push(boundary);
    }
}

fn compiler_plugin_abi_diagnostic(
    error: &clew::error::ClewError,
    context: &RunContext,
) -> Option<Value> {
    if error.code != clew::error::ErrorCode::IncompleteSemanticAnalysis {
        return None;
    }
    let diagnostic = error.evidence.iter().find_map(|encoded| {
        let value: Value = serde_json::from_str(encoded).ok()?;
        (value.get("schema").and_then(Value::as_str)
            == Some("verified-index-failure-diagnostic/0.1"))
        .then_some(value)
    })?;
    let rows = diagnostic.get("workerDiagnostics")?.as_array()?;
    let messages = rows
        .iter()
        .filter_map(|row| row.get("message").and_then(Value::as_str))
        .collect::<Vec<_>>();
    let plugin_failure = messages.iter().any(|message| {
        message.contains("NoClassDefFoundError") || message.contains("ClassNotFoundException")
    }) && messages.iter().any(|message| {
        message.contains("ComponentRegistrar")
            || message.contains("CompilerPluginRegistrar")
            || message.contains("compiler.plugin")
    });
    if !plugin_failure {
        return None;
    }
    let missing_class = messages.iter().find_map(|message| {
        ["NoClassDefFoundError: ", "ClassNotFoundException: "]
            .into_iter()
            .find_map(|marker| {
                message
                    .split_once(marker)
                    .map(|(_, value)| value.split_whitespace().next().unwrap_or(value))
            })
    });
    let manifest = context.provenance.get("semanticInputManifest")?;
    let plugin_artifacts = manifest
        .get("requestedCompilerPlugins")
        .or_else(|| manifest.get("orderedCompilerPlugins"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .filter_map(|identity| {
            let without_digest = identity
                .rsplit_once(":sha256:")
                .map_or(identity, |row| row.0);
            without_digest
                .rsplit(['/', ':'])
                .next()
                .filter(|name| name.ends_with(".jar"))
                .map(str::to_owned)
        })
        .collect::<BTreeSet<_>>()
        .into_iter()
        .take(16)
        .collect::<Vec<_>>();
    Some(json!({
        "category":"COMPILER_PLUGIN_ABI_MISMATCH",
        "declaredCompilerVersion":context.provenance.get("declaredCompilerVersion").cloned().unwrap_or(Value::Null),
        "analyzerCompilerVersion":context.provenance.get("analyzerCompilerVersion").cloned().unwrap_or(Value::Null),
        "missingClass":missing_class.map(|value| value.replace('/', ".")),
        "requestedPluginArtifacts":plugin_artifacts,
        "diagnosticCount":diagnostic.get("workerDiagnosticCount").cloned().unwrap_or(Value::Null),
        "diagnosticsHash":canonical_hash(rows).ok(),
    }))
}

fn emit_terminal(context: &mut RunContext, detail: &str) -> i32 {
    if std::env::var_os("CODECLEW_K1_LOCAL_DIAGNOSTICS").as_deref()
        == Some(std::ffi::OsStr::new("1"))
    {
        eprintln!("{}/{}: {}", context.stage, context.reason_code, detail);
    }
    context.telemetry.external_wall_micros = context.total_start.elapsed().as_micros() as u64;
    let boundary = json!({
        "boundaryId":hash_bytes(format!("{}:{}",context.stage,context.reason_code).as_bytes()),
        "kindUri":format!("codeclew.boundary/kotlin-k1/{}/1",context.reason_code.to_ascii_lowercase().replace('_',"-")),
        "consequence":"PROOF_INVALID",
    });
    if !context
        .boundaries
        .iter()
        .any(|candidate| candidate == &boundary)
    {
        context.boundaries.push(boundary);
    }
    context.telemetry.boundary_count = context.boundaries.len() as u64;
    let serialization_start = Instant::now();
    let terminal = if let Some(detail_digest) = &context.detail_digest_override {
        KotlinAttempt::terminal_with_detail_digest(
            context.status,
            context.stage,
            context.reason_code,
            detail_digest,
            context.selected_inputs.clone(),
            context.snapshot.clone(),
            context.provenance.clone(),
            context.boundaries.clone(),
            context.cache.clone(),
            context.telemetry.clone(),
        )
    } else {
        KotlinAttempt::terminal(
            context.status,
            context.stage,
            context.reason_code,
            detail,
            context.selected_inputs.clone(),
            context.snapshot.clone(),
            context.provenance.clone(),
            context.boundaries.clone(),
            context.cache.clone(),
            context.telemetry.clone(),
        )
    };
    let mut attempt = terminal.unwrap_or_else(|_| fallback_terminal(context));
    context.telemetry.serialization_micros = serialization_start.elapsed().as_micros() as u64;
    attempt.cost.serialization_micros = context.telemetry.serialization_micros;
    attempt.cost.boundary_count = context.telemetry.boundary_count;
    attempt.cost.external_wall_micros = context.total_start.elapsed().as_micros() as u64;
    let _ = attempt.seal_for_stdout();
    if let Some(path) = &context.attempt_output
        && let Err(retention_error) = retain_attempt(path, context.repository.as_deref(), &attempt)
    {
        context.boundaries.push(json!({
            "boundaryId":hash_bytes(b"attempt-retention-failed"),
            "kindUri":"codeclew.boundary/kotlin-k1/attempt-retention-failed/1",
            "consequence":"PROOF_INVALID",
        }));
        context.telemetry.boundary_count = context.boundaries.len() as u64;
        attempt = KotlinAttempt::terminal(
            "FAILED",
            "ATTEMPT_RETENTION",
            "ATTEMPT_RETENTION_FAILED",
            &retention_error.to_string(),
            context.selected_inputs.clone(),
            context.snapshot.clone(),
            context.provenance.clone(),
            context.boundaries.clone(),
            context.cache.clone(),
            context.telemetry.clone(),
        )
        .unwrap_or_else(|_| fallback_terminal(context));
        let _ = attempt.seal_for_stdout();
    }
    let bytes = canonical_bytes(&attempt).unwrap_or_else(|_| {
        b"{\"schema\":\"codeclew.kotlin-real-repository-attempt/0.1\",\"status\":\"FAILED\"}"
            .to_vec()
    });
    let mut stdout = std::io::stdout().lock();
    let _ = stdout.write_all(&bytes);
    let _ = stdout.write_all(b"\n");
    2
}

fn fallback_terminal(context: &RunContext) -> KotlinAttempt {
    KotlinAttempt {
        schema: kotlin_k1::ATTEMPT_SCHEMA.to_owned(),
        status: "FAILED".to_owned(),
        outcome_kind: "TYPED_TERMINAL".to_owned(),
        failure_stage: Some("ATTEMPT_SEAL".to_owned()),
        reason_code: Some("ATTEMPT_SEAL_FAILED".to_owned()),
        detail_digest: Some(hash_bytes(b"attempt sealing failed")),
        selected_inputs: context.selected_inputs.clone(),
        snapshot: context.snapshot.clone(),
        provenance: context.provenance.clone(),
        boundaries: context.boundaries.clone(),
        adapter_output_digest: None,
        evidence_core: None,
        cache: context.cache.clone(),
        cost: context.telemetry.clone(),
        terminal_semantic_digest: hash_bytes(b"attempt sealing failed"),
        attempt_digest: hash_bytes(b"fallback terminal"),
    }
}

fn main_kotlin_source_files(sources: &[SourceInput]) -> Vec<String> {
    let mut files = sources
        .iter()
        .filter_map(|source| {
            let path = Path::new(&source.normalized_path);
            if path.extension().and_then(|value| value.to_str()) != Some("kt") {
                return None;
            }
            let components = path
                .components()
                .filter_map(|component| match component {
                    Component::Normal(value) => value.to_str(),
                    _ => None,
                })
                .collect::<Vec<_>>();
            components
                .windows(3)
                .any(|window| window == ["src", "main", "kotlin"])
                .then(|| source.normalized_path.clone())
        })
        .collect::<Vec<_>>();
    files.sort();
    files.dedup();
    files
}

fn verify_source_syntax_snapshot_unchanged(
    repo: &Path,
    ignored_components: &BTreeSet<String>,
    expected_sources: &[SourceInput],
    expected_tree_digest: &str,
    expected_vcs_revision: Option<&str>,
    expected_dirty: bool,
    expected_git_status_digest: Option<&str>,
    expected_tracked_symlinks: &[Value],
) -> Result<u64> {
    let started = Instant::now();
    let exclusions = repository_exclusions(repo, ignored_components);
    let current_tracked_symlinks = validate_repository_tree_paths(repo, &exclusions)?;
    if canonical_bytes(&current_tracked_symlinks)?
        != canonical_bytes(&expected_tracked_symlinks.to_vec())?
    {
        anyhow::bail!("tracked symlink object state changed during source-syntax fallback");
    }
    let (current_sources, _, _) = snapshot_repository(repo, &exclusions)?;
    if canonical_bytes(&current_sources)? != canonical_bytes(&expected_sources.to_vec())?
        || k1_repository_tree_digest(&current_sources, &current_tracked_symlinks)?
            != expected_tree_digest
    {
        anyhow::bail!("source/build snapshot changed during source-syntax fallback");
    }
    if git_revision(repo)?.as_deref() != expected_vcs_revision || git_dirty(repo)? != expected_dirty
    {
        anyhow::bail!("repository revision or dirty state changed during source-syntax fallback");
    }
    if let Some(expected) = expected_git_status_digest
        && git_status_digest(repo)?.as_deref() != Some(expected)
    {
        anyhow::bail!("repository Git status changed during source-syntax fallback");
    }
    Ok(started.elapsed().as_micros() as u64)
}

fn source_syntax_failure_class(error: &ClewError) -> Result<String> {
    serde_json::to_value(&error.code)?
        .as_str()
        .map(str::to_owned)
        .context("source-syntax fallback error code is not a string")
}

#[allow(clippy::too_many_arguments)]
fn source_syntax_agent_projection(
    args: &Args,
    facts: &Value,
    sources: &[SourceInput],
    requested_files: &[String],
    adapter_binary: &str,
    repository_tree_digest: &str,
    vcs_revision: Option<&str>,
    dirty: bool,
    failure_stage: &str,
    failure: &ClewError,
    syntax_index_micros: u64,
    source_bytes_read: u64,
    total_wall_micros: u64,
) -> Result<Value> {
    if facts.get("analysisMode").and_then(Value::as_str) != Some("SYNTAX_DECLARATIONS")
        || facts.get("k2Validated").and_then(Value::as_bool) != Some(false)
        || facts.get("partial").and_then(Value::as_bool) != Some(!requested_files.is_empty())
    {
        anyhow::bail!("source-syntax projection received semantic or partial provider facts");
    }
    let source_by_path = sources
        .iter()
        .map(|source| (source.normalized_path.as_str(), source))
        .collect::<BTreeMap<_, _>>();
    let requested_set = requested_files
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    let provider_files = facts
        .get("files")
        .and_then(Value::as_array)
        .context("source-syntax provider has no files")?;
    let mut seen_files = BTreeSet::new();
    let mut syntax_files = Vec::with_capacity(provider_files.len());
    let mut declaration_count = 0u64;
    let mut import_count = 0u64;
    for file in provider_files {
        let path = required_string(file, "/path")?;
        if !requested_set.contains(path) || !seen_files.insert(path.to_owned()) {
            anyhow::bail!("source-syntax provider file set differs from the exact request");
        }
        let source = source_by_path
            .get(path)
            .context("source-syntax provider file is outside the exact snapshot")?;
        if file.get("contentHash").and_then(Value::as_str) != Some(source.content_digest.as_str()) {
            anyhow::bail!("source-syntax provider content differs from the exact snapshot");
        }
        let mut imports = file
            .get("imports")
            .and_then(Value::as_array)
            .context("source-syntax provider file has no imports")?
            .iter()
            .map(|value| {
                value
                    .as_str()
                    .map(str::to_owned)
                    .context("source-syntax import is not a string")
            })
            .collect::<Result<Vec<_>>>()?;
        imports.sort();
        imports.dedup();
        import_count = import_count.saturating_add(imports.len() as u64);
        let declarations = file
            .get("declarations")
            .and_then(Value::as_array)
            .context("source-syntax provider file has no declarations")?
            .iter()
            .map(|declaration| {
                let kind = required_string(declaration, "/kind")?;
                let name = declaration
                    .get("name")
                    .and_then(Value::as_str)
                    .context("source-syntax declaration has no name")?;
                let origin = declaration
                    .get("sourceOrigin")
                    .context("source-syntax declaration has no source origin")?;
                if origin.get("file").and_then(Value::as_str) != Some(path) {
                    anyhow::bail!("source-syntax declaration origin differs from its file");
                }
                let start = origin
                    .get("rangeStart")
                    .and_then(Value::as_u64)
                    .context("source-syntax declaration has no range start")?;
                let end = origin
                    .get("rangeEnd")
                    .and_then(Value::as_u64)
                    .context("source-syntax declaration has no range end")?;
                Ok(json!({
                    "kind":kind,
                    "name":name,
                    "sourceOrigin":{
                        "file":path,
                        "rangeStart":start,
                        "rangeEnd":end,
                        "rangeUnit":"UTF16_CODE_UNIT",
                    },
                }))
            })
            .collect::<Result<Vec<_>>>()?;
        declaration_count = declaration_count.saturating_add(declarations.len() as u64);
        syntax_files.push(json!({
            "path":path,
            "imports":imports,
            "declarations":declarations,
        }));
    }
    if seen_files
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>()
        != requested_set
    {
        anyhow::bail!("source-syntax provider omitted an exact requested file");
    }
    syntax_files.sort_by(|left, right| {
        left.get("path")
            .and_then(Value::as_str)
            .cmp(&right.get("path").and_then(Value::as_str))
    });

    let failure_class = source_syntax_failure_class(failure)?;
    let boundary_id = canonical_hash(&json!({
        "kind":"toolchain-or-semantic-unavailable",
        "repositoryTreeDigest":repository_tree_digest,
        "compilation":args.compilation,
        "failureStage":failure_stage,
        "failureClass":failure_class,
    }))?;
    let boundary = json!({
        "boundaryId":boundary_id,
        "kindUri":"codeclew.boundary/kotlin/toolchain-or-semantic-unavailable/1",
        "consequence":"SEMANTIC_TOPOLOGY_AND_COMPILATION_MEMBERSHIP_UNKNOWN",
        "origin":Value::Null,
        "provider":"KOTLIN_SOURCE_SYNTAX_FALLBACK",
        "applicability":global_boundary_applicability(BOUNDARY_EFFECT_TOPOLOGY),
        "details":{
            "failureStage":failure_stage,
            "failureClass":failure_class,
            "fallbackAuthority":"SOURCE_SYNTAX",
            "analysisMode":"SYNTAX_DECLARATIONS",
            "k2Validated":false,
            "compilationMembership":"UNKNOWN",
            "semanticCacheReuse":"FORBIDDEN",
        },
    });
    let boundaries = vec![boundary];
    let boundary_set_digest = canonical_hash(&boundaries)?;
    let query_specification = impact_query_specification();
    let query_specification_digest = canonical_hash(&query_specification)?;
    let empty_path_facts = Vec::<Value>::new();
    let syntax_digest = canonical_hash(&syntax_files)?;
    let requested_file_set_digest = canonical_hash(&requested_files)?;
    Ok(json!({
        "schema":"codeclew.agent-impact-projection/0.2",
        "adapter":{
            "adapterId":"codeclew.kotlin-source-syntax",
            "version":"0.1.0",
            "binaryDigest":adapter_binary,
            "languageId":"kotlin",
        },
        "snapshot":{
            "repositoryTreeDigest":repository_tree_digest,
            "vcsRevision":vcs_revision,
            "dirty":dirty,
            "requestedCompilation":args.compilation,
            "targets":[{
                "requestedTargetId":args.compilation,
                "compilationMembership":"UNKNOWN",
                "platform":"JVM",
            }],
        },
        "compiler":{
            "method":"SOURCE_SYNTAX_DECLARATIONS",
            "status":"SEMANTIC_UNAVAILABLE",
            "grade":"SOURCE_CHECKED",
            "k2Validated":false,
        },
        "projectionAuthority":{
            "schema":"codeclew.kotlin-source-syntax-authority/0.1",
            "authority":"SOURCE_SYNTAX",
            "analysisMode":"SYNTAX_DECLARATIONS",
            "k2Validated":false,
            "semanticCacheReuse":"FORBIDDEN",
            "repositoryTreeDigest":repository_tree_digest,
            "requestedCompilation":args.compilation,
            "compilationMembership":"UNKNOWN",
            "requestedFileSetDigest":requested_file_set_digest,
            "syntaxProjectionDigest":syntax_digest,
        },
        "impact":{
            "schema":"codeclew.impact-result/0.1",
            "status":"PARTIAL_BOUNDARY",
            "closureSpecification":evidence_adapters::IMPACT_CLOSURE_SPEC,
            "querySpecification":query_specification,
            "seedEntity":Value::Null,
            "maxDepth":args.max_depth,
            "maxEntities":args.max_entities,
            "affected":[],
            "paths":[],
            "mandatoryObligations":[{
                "id":"impact-closure-completeness",
                "kind":"codeclew.obligation/impact-closure-completeness/1",
                "mandatory":true,
                "status":"UNKNOWN",
                "reason":"TOOLCHAIN_OR_SEMANTIC_UNAVAILABLE",
                "evidenceFactIds":[],
                "providerPayload":{
                    "boundaryId":boundary_id,
                    "queryRelevantBoundaryCount":1,
                    "queryRelevantBoundarySetDigest":boundary_set_digest,
                },
            }],
            "boundaries":boundaries,
            "boundaryAssessment":{
                "schema":"codeclew.query-boundary-assessment/0.1",
                "policy":"EXPLICIT_APPLICABILITY_FAIL_CLOSED",
                "querySpecificationDigest":query_specification_digest,
                "projectBoundaryCount":1,
                "projectBoundarySetDigest":boundary_set_digest,
                "projectGlobalBoundaryCount":1,
                "queryRelevantProjectBoundaryCount":1,
                "queryLocalBoundaryCount":0,
                "queryRelevantBoundaryCount":1,
                "queryRelevantBoundarySetDigest":boundary_set_digest,
            },
            "pathProof":{
                "schema":"codeclew.displayed-path-proof/0.1",
                "status":"SATISFIED",
                "claim":"DISPLAYED_SEMANTIC_PATH_SET_IS_EMPTY",
                "pathCount":0,
                "pathFactCount":0,
                "pathFactSetDigest":canonical_hash(&empty_path_facts)?,
                "pathFactIds":[],
                "querySpecificationDigest":query_specification_digest,
            },
            "providerPayload":{
                "requestedSeedEntity":args.seed_entity,
                "selectionAuthority":"UNRESOLVED_WITHOUT_COMPILER_SEMANTICS",
            },
        },
        "selectedEntities":[],
        "pathFacts":empty_path_facts,
        "semanticFacts":[],
        "semanticOutputDigest":Value::Null,
        "syntax":{
            "schema":"codeclew.kotlin-source-syntax-projection/0.1",
            "authority":"SOURCE_SYNTAX",
            "analysisMode":"SYNTAX_DECLARATIONS",
            "k2Validated":false,
            "coverage":"EXACT_REPOSITORY_MAIN_KOTLIN_CANDIDATES",
            "requestedCompilation":args.compilation,
            "compilationMembership":"UNKNOWN",
            "fileCount":syntax_files.len(),
            "declarationCount":declaration_count,
            "importCount":import_count,
            "repositoryMainKotlinCandidates":syntax_files,
        },
        "cost":{
            "totalWallMicros":total_wall_micros,
            "syntaxIndexMicros":syntax_index_micros,
            "sourceBytesRead":source_bytes_read,
        },
    }))
}

#[allow(clippy::too_many_arguments)]
fn source_syntax_partial_fallback(
    mut worker: WorkerClient,
    args: &Args,
    context: &mut RunContext,
    repo: &Path,
    ignored_components: &BTreeSet<String>,
    snapshot_sources: &[SourceInput],
    repository_tree_digest: &str,
    vcs_revision: Option<&str>,
    dirty: bool,
    initial_git_status_digest: Option<&str>,
    tracked_symlinks: &[Value],
    adapter_binary: &str,
    failure_stage: &'static str,
    failure: &ClewError,
    source_bytes_read: u64,
) -> Result<RunSuccess> {
    if !permits_source_syntax_fallback(args) || !source_syntax_fallback_error(failure) {
        return Err(failure.clone().into());
    }
    let compiler_index_profile = context.cache.get("compilerIndex").cloned();
    context.enter(
        "SOURCE_SYNTAX_FALLBACK",
        "FAILED",
        "SOURCE_SYNTAX_FALLBACK_FAILED",
    );
    let before_snapshot_micros = verify_source_syntax_snapshot_unchanged(
        repo,
        ignored_components,
        snapshot_sources,
        repository_tree_digest,
        vcs_revision,
        dirty,
        initial_git_status_digest,
        tracked_symlinks,
    )?;
    let requested_files = main_kotlin_source_files(snapshot_sources);
    let syntax_started = Instant::now();
    let verified = worker.index_files_source_syntax_verified(&json!({
        "repo":repo,
        "compilation":args.compilation,
        "syntaxOnly":true,
        "files":requested_files,
    }))?;
    let syntax_index_micros = syntax_started.elapsed().as_micros() as u64;
    let facts = worker.inspect_verified_source_syntax(&verified)?;
    let mut projection = source_syntax_agent_projection(
        args,
        facts,
        snapshot_sources,
        &requested_files,
        adapter_binary,
        repository_tree_digest,
        vcs_revision,
        dirty,
        failure_stage,
        failure,
        syntax_index_micros,
        source_bytes_read,
        context.total_start.elapsed().as_micros() as u64,
    )?;
    let profile = worker.last_profile.clone();
    worker.shutdown()?;
    let after_snapshot_micros = verify_source_syntax_snapshot_unchanged(
        repo,
        ignored_components,
        snapshot_sources,
        repository_tree_digest,
        vcs_revision,
        dirty,
        initial_git_status_digest,
        tracked_symlinks,
    )?;
    projection["cost"]["totalWallMicros"] =
        Value::from(context.total_start.elapsed().as_micros() as u64);
    projection["cost"]["snapshotVerificationMicros"] =
        Value::from(before_snapshot_micros.saturating_add(after_snapshot_micros));
    context.telemetry.source_hashing_micros = context
        .telemetry
        .source_hashing_micros
        .saturating_add(before_snapshot_micros)
        .saturating_add(after_snapshot_micros);
    context.telemetry.cold_index_micros = syntax_index_micros;
    context.telemetry.provider_processing_micros = context
        .telemetry
        .provider_processing_micros
        .saturating_add(profile.worker_processing_micros);
    context.telemetry.fact_count = 0;
    context.telemetry.boundary_count = 1;
    context.boundaries = projection
        .pointer("/impact/boundaries")
        .and_then(Value::as_array)
        .cloned()
        .context("source-syntax projection has no boundary")?;
    context.cache = json!({
        "status":"BYPASSED_SOURCE_SYNTAX_FALLBACK",
        "hit":false,
        "semanticCacheReuse":"FORBIDDEN",
    });
    if let Some(compiler_index_profile) = compiler_index_profile {
        context.cache["compilerIndex"] = compiler_index_profile;
    }
    context.provenance["fallbackAuthority"] = json!({
        "authority":"SOURCE_SYNTAX",
        "analysisMode":"SYNTAX_DECLARATIONS",
        "k2Validated":false,
        "triggerStage":failure_stage,
        "triggerClass":source_syntax_failure_class(failure)?,
    });
    context.enter("COMPLETE", "FAILED", "UNREACHABLE_COMPLETE_FAILURE");
    Ok(RunSuccess {
        output: None,
        agent_projection: Some(projection),
        core: None,
        agent_output: true,
    })
}

fn run(args: Args, context: &mut RunContext) -> Result<RunSuccess> {
    let total_start = Instant::now();
    context.enter("RUN_PHASE", "REFUSED", "INVALID_PAIRED_RUN_PHASE");
    let prepared_refusal_mode = validate_prepared_refusal_cli(&args)?;
    if prepared_refusal_mode {
        validate_prepared_refusal_phase(args.run_phase, args.seed_entity.as_deref())?;
    } else {
        validate_run_phase(
            args.run_phase,
            args.seed_entity.as_deref(),
            args.state_root.is_some(),
        )?;
    }
    context.enter("REPOSITORY_NORMALIZATION", "REFUSED", "INVALID_REPOSITORY");
    let repo = normalize_repo(&args.repo)?;
    context.repository = Some(repo.clone());
    if let Some(path) = &args.attempt_output {
        context.enter(
            "ATTEMPT_DESTINATION_PREFLIGHT",
            "REFUSED",
            "ATTEMPT_RETENTION_FAILED",
        );
        validate_attempt_destination(path, Some(&repo))?;
    }
    if prepared_refusal_mode {
        consume_prepared_refusal(&args, &repo, context)?;
        anyhow::bail!("validated dependency-preparation refusal");
    }
    context.enter("BUILD_STATE_CONFIGURATION", "FAILED", "BUILD_STATE_INVALID");
    let build_state = args
        .build_state_root
        .as_deref()
        .map(|root| validate_build_state_root(root, &repo))
        .transpose()?;
    let build_state_identity = build_state
        .as_ref()
        .map(PreparedBuildStateIdentity::semantic_identity)
        .unwrap_or_else(|| {
            json!({
                "mode":"REPOSITORY_LOCAL_OFFLINE",
                "authority":"UNSEALED_DEVELOPMENT_CACHE",
            })
        });
    context.provenance["buildStateIdentity"] = build_state_identity.clone();
    context.selected_inputs["buildState"] = build_state_identity;
    let ignored_components = [
        ".git",
        ".gradle",
        ".kotlin",
        ".semantic-thread",
        "build",
        "target",
        "node_modules",
        ".idea",
        ".vscode",
    ]
    .into_iter()
    .map(ToOwned::to_owned)
    .collect();
    context.enter("SOURCE_SNAPSHOT", "FAILED", "SOURCE_SNAPSHOT_FAILED");
    let exclusions = repository_exclusions(&repo, &ignored_components);
    let tracked_symlinks = validate_repository_tree_paths(&repo, &exclusions)?;
    let (mut sources, _, snapshot_micros) = snapshot_repository(&repo, &exclusions)?;
    let mut repository_tree_digest = k1_repository_tree_digest(&sources, &tracked_symlinks)?;
    let tracked_symlink_manifest_digest = tracked_symlink_manifest_digest(&tracked_symlinks)?;
    let mut source_bytes_read = sources.iter().map(|source| source.size_bytes).sum();
    let source_syntax_snapshot_sources = sources.clone();
    let source_syntax_repository_tree_digest = repository_tree_digest.clone();
    let source_syntax_source_bytes_read = source_bytes_read;
    context.telemetry.source_hashing_micros = snapshot_micros;
    context.telemetry.source_bytes_read = source_bytes_read;
    let vcs_revision = git_revision(&repo)?;
    let dirty = git_dirty(&repo)?;
    // External sealed runs retain the full Git/index/worktree observation.
    // Direct development mode already snapshots every semantic source and
    // build input before/after; observing mutable ignored build caches as Git
    // extras would turn normal offline Gradle operation into a false source
    // mutation.
    let initial_git_status_digest = if build_state.is_some() {
        git_status_digest(&repo)?
    } else {
        None
    };
    context.snapshot = json!({
        "repositoryTreeDigest":repository_tree_digest,
        "vcsRevision":vcs_revision,
        "dirty":dirty,
        "trackedSymlinkManifestDigest":tracked_symlink_manifest_digest,
    });
    context.provenance["trackedSymlinkManifestDigest"] =
        Value::String(tracked_symlink_manifest_digest.clone());

    context.enter("ADAPTER_IDENTITY", "FAILED", "ADAPTER_IDENTITY_FAILED");
    let adapter_binary = executable_digest()?;
    context.provenance["adapterBinaryDigest"] = Value::String(adapter_binary.clone());

    context.enter(
        "CACHE_INITIALIZATION",
        "FAILED",
        "CACHE_INITIALIZATION_FAILED",
    );
    let semantic_cache = args
        .state_root
        .as_deref()
        .map(|root| SemanticCache::open(root, &repo))
        .transpose()?;
    if let (Some(cache), Some(build_state)) = (&semantic_cache, &build_state) {
        ensure_external_roots_disjoint(cache.canonical_root(), &build_state.root)?;
    }
    context.cache = if let Some(cache) = &semantic_cache {
        json!({
            "status":"LOOKUP_PENDING",
            "hit":false,
            "externalStateRootIdentity":cache.root_digest(),
            "payloadCostScope":"ORIGINAL_COMPILER_ISSUANCE",
            "attemptCostScope":"CURRENT_INVOCATION",
        })
    } else {
        json!({"status":"DISABLED","hit":false,"reason":"NO_STATE_ROOT"})
    };

    let adapter_start = Instant::now();
    if permits_relaxed_agent_graph_lookup(&args)
        && let Some(requested_seed) = args.seed_entity.as_deref()
        && let Some(cache) = &semantic_cache
    {
        context.enter("AGENT_GRAPH_LOOKUP", "FAILED", "AGENT_GRAPH_LOOKUP_FAILED");
        context.telemetry.cache_requests += 1;
        if let Some(lookup) = cache.lookup_agent_graph_query(
            &repository_tree_digest,
            vcs_revision.as_deref(),
            dirty,
            &args.compilation,
            "codeclew.kotlin-k2",
            "0.1.0",
            build_state
                .as_ref()
                .map(|identity| identity.seed_digest.as_str()),
        )? {
            let after_exclusions = repository_exclusions(&repo, &ignored_components);
            let after_symlinks = validate_repository_tree_paths(&repo, &after_exclusions)?;
            let (after_sources, _, after_snapshot_micros) =
                snapshot_repository(&repo, &after_exclusions)?;
            context.telemetry.source_hashing_micros = context
                .telemetry
                .source_hashing_micros
                .saturating_add(after_snapshot_micros);
            let git_changed = if let Some(expected) = initial_git_status_digest.as_ref() {
                git_status_digest(&repo)?.as_ref() != Some(expected)
            } else {
                false
            };
            if after_symlinks != tracked_symlinks
                || k1_repository_tree_digest(&after_sources, &after_symlinks)?
                    != repository_tree_digest
                || git_changed
            {
                anyhow::bail!("repository changed during early agent graph lookup");
            }
            let authority = lookup.index.projection_authority().clone();
            let project_summary = lookup.index.project_boundary_summary();
            let projection = indexed_agent_impact_projection(
                &lookup,
                requested_seed,
                args.max_depth,
                args.max_entities,
                &repository_tree_digest,
                vcs_revision.as_deref(),
                dirty,
                &args.compilation,
            )?;
            context.telemetry.cache_hits += 1;
            context.telemetry.cache_bytes_read = lookup.bytes_read;
            context.telemetry.store_read_micros = lookup.read_micros;
            context.telemetry.warm_index_micros = lookup.read_micros;
            context.telemetry.fact_count = projection
                .get("pathFacts")
                .and_then(Value::as_array)
                .map_or(0, |facts| facts.len() as u64);
            context.telemetry.boundary_count = project_summary
                .get("count")
                .and_then(Value::as_u64)
                .unwrap_or(0);
            context.telemetry.query_projection_micros = projection
                .pointer("/impact/queryMicros")
                .and_then(Value::as_u64)
                .unwrap_or(0);
            context.cache = json!({
                "status":"VERIFIED_AGENT_GRAPH_HIT",
                "hit":true,
                "externalStateRootIdentity":cache.root_digest(),
                "bytesRead":lookup.bytes_read,
                "readMicros":lookup.read_micros,
                "legacyObjectsScanned":lookup.index_telemetry.legacy_objects_scanned,
                "legacyBytesScanned":lookup.index_telemetry.legacy_bytes_scanned,
                "migrationBytesWritten":lookup.index_telemetry.migration_bytes_written,
                "semanticOutputDigest":authority.get("semanticOutputDigest"),
                "semanticFactsDigest":authority.get("semanticFactsDigest"),
                "projectBoundarySummary":project_summary,
            });
            context.provenance["cachedProjectionAuthority"] = authority;
            context.boundaries = projection
                .pointer("/impact/boundaries")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            context.enter("COMPLETE", "FAILED", "UNREACHABLE_COMPLETE_FAILURE");
            return Ok(RunSuccess {
                output: None,
                agent_projection: Some(projection),
                core: None,
                agent_output: true,
            });
        }
        context.cache["status"] = Value::String("EARLY_AGENT_GRAPH_MISS".to_owned());
    }

    context.enter("WORKER_START", "FAILED", "WORKER_START_FAILED");
    let worker_start = Instant::now();
    let mut worker = WorkerClient::start_with_states(
        &workspace_root(),
        build_state.as_ref().map(|identity| identity.root.as_path()),
        semantic_cache
            .as_ref()
            .map(SemanticCache::compiler_index_root),
    )?;
    context.telemetry.adapter_startup_micros = worker_start.elapsed().as_micros() as u64;
    context.enter("BUILD_DISCOVERY", "REFUSED", "BUILD_DISCOVERY_FAILED");
    let build_start = Instant::now();
    let project = match worker.request(
        RequestKind::OpenProject,
        &json!({"repo":repo,"compilation":args.compilation}),
    ) {
        Ok(project) => project,
        Err(error)
            if permits_source_syntax_fallback(&args) && source_syntax_fallback_error(&error) =>
        {
            return source_syntax_partial_fallback(
                worker,
                &args,
                context,
                &repo,
                &ignored_components,
                &source_syntax_snapshot_sources,
                &source_syntax_repository_tree_digest,
                vcs_revision.as_deref(),
                dirty,
                initial_git_status_digest.as_deref(),
                &tracked_symlinks,
                &adapter_binary,
                "BUILD_DISCOVERY",
                &error,
                source_syntax_source_bytes_read,
            );
        }
        Err(error) => return Err(error.into()),
    };
    let build_discovery_micros = build_start.elapsed().as_micros() as u64;
    let build_profile = worker.last_profile.clone();
    context.telemetry.build_discovery_micros = build_discovery_micros;
    context.telemetry.provider_processing_micros = build_profile.worker_processing_micros;

    // OpenProject may switch from the bootstrap 2.4 worker to the exact
    // project minor line. Cache identity must therefore be read only after
    // that live switch has completed.
    let trusted_distribution = worker
        .trusted_distribution_identity()
        .context("trusted worker distribution identity is unavailable")?;
    let distribution_value = json!({
        "treeHash":trusted_distribution.tree_hash,
        "buildInputDigest":trusted_distribution.build_input_digest,
        "pluginFingerprint":trusted_distribution.plugin_fingerprint,
    });
    context.provenance["trustedWorkerDistribution"] = distribution_value.clone();

    let raw_semantic_input_manifest = project
        .get("semanticInputManifest")
        .context("OpenProject has no exact semanticInputManifest")?;
    match &build_state {
        Some(identity) => {
            validate_worker_build_state_identity(raw_semantic_input_manifest, identity)?
        }
        None => validate_repository_local_build_state_identity(raw_semantic_input_manifest)?,
    }
    let raw_semantic_input_manifest_hash = required_string(&project, "/semanticInputManifestHash")?;
    if canonical_hash(raw_semantic_input_manifest)? != raw_semantic_input_manifest_hash {
        anyhow::bail!("OpenProject semantic input manifest hash differs from its body");
    }
    let semantic_input_manifest = stable_semantic_input_manifest(raw_semantic_input_manifest)?;
    let semantic_input_manifest_hash = canonical_hash(&semantic_input_manifest)?;
    let build_model_digest = stable_project_model_digest(
        &project,
        &semantic_input_manifest,
        &semantic_input_manifest_hash,
    )?;
    let generated_start = Instant::now();
    let generated_boundaries = augment_generated_sources(&repo, &project, &mut sources)?;
    sources.sort_by(|left, right| left.artifact_id.cmp(&right.artifact_id));
    repository_tree_digest = k1_repository_tree_digest(&sources, &tracked_symlinks)?;
    source_bytes_read = sources.iter().map(|source| source.size_bytes).sum();
    context.telemetry.source_hashing_micros = context
        .telemetry
        .source_hashing_micros
        .saturating_add(generated_start.elapsed().as_micros() as u64);
    context.telemetry.source_bytes_read = source_bytes_read;
    context.snapshot["repositoryTreeDigest"] = Value::String(repository_tree_digest.clone());

    let mut build_model_boundaries = project_boundaries(&project)?;
    if build_state.is_none() {
        build_model_boundaries.push(json!({
            "boundaryId":canonical_hash(&json!({"kind":"repository-local-build-state","compilation":args.compilation}))?,
            "kindUri":"codeclew.boundary/kotlin/repository-local-build-state/1",
            "consequence":"ENUMERATION_INCOMPLETE",
            "origin":Value::Null,
            "provider":"KOTLIN_PROJECT_MODEL",
            "applicability":global_boundary_applicability(BOUNDARY_EFFECT_BUILD_FIDELITY),
            "details":{
                "mode":"LEGACY_REPOSITORY_OWNED",
                "cacheReuse":"ALLOWED_ONLY_WHILE_SOURCE_AND_BUILD_NAMESPACE_MATCH",
            },
        }));
    }
    build_model_boundaries.extend(generated_boundaries.clone());
    build_model_boundaries.sort_by(|left, right| {
        left["boundaryId"]
            .as_str()
            .cmp(&right["boundaryId"].as_str())
    });
    build_model_boundaries.dedup_by(|left, right| left["boundaryId"] == right["boundaryId"]);
    context.boundaries.extend(build_model_boundaries.clone());
    context.snapshot["buildModelDigest"] = Value::String(build_model_digest.clone());
    context.snapshot["semanticInputManifestHash"] =
        Value::String(semantic_input_manifest_hash.to_owned());
    context.provenance["semanticInputManifestHash"] =
        Value::String(semantic_input_manifest_hash.to_owned());
    context.provenance["semanticInputManifest"] = semantic_input_manifest.clone();
    context.provenance["declaredCompilerVersion"] = semantic_input_manifest
        .get("declaredCompilerVersion")
        .cloned()
        .unwrap_or(Value::Null);
    context.provenance["analyzerCompilerVersion"] = project
        .get("workerCompilerVersion")
        .cloned()
        .unwrap_or(Value::Null);
    let range_validator = SourceRangeValidator::new(&repo, &sources)?;

    context.enter("CACHE_KEY", "FAILED", "CACHE_KEY_INVALID");
    let cache_key = CacheKey::exact(
        &repository_tree_digest,
        vcs_revision.as_deref(),
        dirty,
        &args.compilation,
        &adapter_binary,
        "0.1.0",
        distribution_value,
        json!({
            "language":worker.capabilities.language,
            "compilerVersion":worker.capabilities.compiler_version,
            "workerVersion":worker.capabilities.worker_version,
            "protocolVersions":worker.capabilities.protocol_versions.iter().map(|version| json!({"major":version.major,"minor":version.minor})).collect::<Vec<_>>(),
            "supportedOperations":worker.capabilities.supported_operations,
            "supportedLanguageFeatures":worker.capabilities.supported_language_features,
            "unsupportedFeatures":worker.capabilities.unsupported_features,
        }),
        &semantic_input_manifest,
        &semantic_input_manifest_hash,
    )?;
    context.cache["keyDigest"] = Value::String(cache_key.digest.clone());
    if let Some(cache) = &semantic_cache {
        context.enter("CACHE_REVALIDATION", "FAILED", "CACHE_REVALIDATION_FAILED");
        context.telemetry.cache_requests += 1;
        match cache.lookup(&cache_key)? {
            CacheLookup::Hit(hit) => {
                if matches!(args.run_phase, RunPhase::Cold) {
                    context.enter("CACHE_REVALIDATION", "FAILED", "COLD_CACHE_NOT_FRESH");
                    anyhow::bail!("cold run requires a fresh semantic cache namespace");
                }
                context.telemetry.cache_hits += 1;
                context.telemetry.cache_bytes_read = hit.bytes_read;
                context.telemetry.store_read_micros = hit.read_micros;
                context.telemetry.warm_index_micros = hit.read_micros;
                context.cache["status"] = Value::String("VERIFIED_HIT".to_owned());
                context.cache["hit"] = Value::Bool(true);
                context.cache["cachedSemanticOutputDigest"] =
                    Value::String(kotlin_k1::semantic_output_digest(&hit.output)?);
                context.cache["semanticFactsDigest"] =
                    Value::String(kotlin_k1::semantic_facts_digest(&hit.output)?);
                let _ = worker.shutdown();
                verify_repository_unchanged(
                    &repo,
                    &ignored_components,
                    &project,
                    &sources,
                    &repository_tree_digest,
                    initial_git_status_digest.as_deref(),
                    &generated_boundaries,
                    &tracked_symlinks,
                )?;
                let mut output = hit.output;
                let seed = resolve_requested_seed(
                    args.seed_entity
                        .as_deref()
                        .context("warm run has no explicit seed")?,
                    &output.entities,
                    &output.facts,
                )?;
                let query_start = Instant::now();
                output.impact = deterministic_impact(
                    &seed,
                    &output.entities,
                    &output.facts,
                    &output.boundaries,
                    args.max_depth,
                    args.max_entities,
                    "CALLER_SELECTED_RESOLVED_ENTITY",
                )?;
                let query_micros = query_start.elapsed().as_micros() as u64;
                output.impact["queryMicros"] = Value::from(query_micros);
                context.selected_inputs["query"]["proposedSeedEntity"] = Value::String(seed);
                context.selected_inputs["query"]["selectionAuthority"] =
                    Value::String("CALLER_SELECTED_RESOLVED_ENTITY".to_owned());
                context.telemetry.query_projection_micros = query_micros;
                output.cost.total_wall_micros = context.total_start.elapsed().as_micros() as u64;
                output.cost.repository_snapshot_micros = context.telemetry.source_hashing_micros;
                output.cost.build_discovery_micros = build_discovery_micros;
                output.cost.cold_index_micros = 0;
                output.cost.warm_index_micros = hit.read_micros;
                output.cost.adapter_micros = adapter_start.elapsed().as_micros() as u64;
                output.cost.query_micros = query_micros;
                output.cost.source_bytes_read = source_bytes_read;
                output.cost.cache_requests = build_profile.cache_requests.saturating_add(1);
                output.cost.cache_hits = build_profile.cache_hits.saturating_add(1);
                output.cost.provider_processing_micros =
                    context.telemetry.provider_processing_micros;
                context.telemetry.cache_requests = output.cost.cache_requests;
                context.telemetry.cache_hits = output.cost.cache_hits;
                validate_adapter_ranges(&output, &range_validator)?;
                output.seal()?;
                let core = if args.agent_output && context.attempt_output.is_none() {
                    None
                } else {
                    Some(kotlin_k1::validate_kotlin_core_binding(&output)?)
                };
                context.cache["semanticOutputDigest"] =
                    Value::String(kotlin_k1::semantic_output_digest(&output)?);
                context.telemetry.stored_fact_bytes = output.cost.stored_fact_bytes;
                context.telemetry.fact_count = output.facts.len() as u64;
                context.telemetry.boundary_count = output.boundaries.len() as u64;
                context.boundaries = output.boundaries.clone();
                return Ok(RunSuccess {
                    output: Some(output),
                    agent_projection: None,
                    core,
                    agent_output: args.agent_output,
                });
            }
            CacheLookup::Miss { read_micros } => {
                if matches!(args.run_phase, RunPhase::Warm) {
                    context.enter("CACHE_REVALIDATION", "FAILED", "WARM_CACHE_MISS");
                    anyhow::bail!("warm run requires a verified semantic cache hit");
                }
                context.telemetry.store_read_micros = read_micros;
                context.cache["status"] = Value::String("MISS".to_owned());
            }
        }
    }

    context.enter("K2_INDEX", "PARTIAL", "K2_INDEX_FAILED");
    let index_start = Instant::now();
    let index_result = worker.index_files_verified(
        &json!({"repo":repo,"compilation":args.compilation,"syntaxOnly":false}),
    );
    let profile = worker.last_profile.clone();
    retain_operational_profiles(context, &profile);
    let verified = match index_result {
        Ok(verified) => verified,
        Err(error)
            if permits_source_syntax_fallback(&args) && source_syntax_fallback_error(&error) =>
        {
            return source_syntax_partial_fallback(
                worker,
                &args,
                context,
                &repo,
                &ignored_components,
                &source_syntax_snapshot_sources,
                &source_syntax_repository_tree_digest,
                vcs_revision.as_deref(),
                dirty,
                initial_git_status_digest.as_deref(),
                &tracked_symlinks,
                &adapter_binary,
                "K2_INDEX",
                &error,
                source_syntax_source_bytes_read,
            );
        }
        Err(error) => return Err(error.into()),
    };
    let facts = worker.inspect_verified_index(&verified)?;
    if facts.get("k2Validated").and_then(Value::as_bool) != Some(true) {
        anyhow::bail!("worker index did not retain explicit k2Validated=true");
    }
    if facts
        .get("semanticInputManifestHash")
        .and_then(Value::as_str)
        != Some(raw_semantic_input_manifest_hash)
        || facts.get("semanticInputManifest") != Some(raw_semantic_input_manifest)
        || facts.get("projectModelHash") != project.get("projectModelHash")
    {
        anyhow::bail!(
            "K2 facts differ from the exact OpenProject semantic manifest used for the cache key"
        );
    }
    let index_micros = index_start.elapsed().as_micros() as u64;
    context.telemetry.cold_index_micros = index_micros;
    context.telemetry.provider_processing_micros = context
        .telemetry
        .provider_processing_micros
        .saturating_add(profile.worker_processing_micros);
    context.telemetry.dependency_verification_micros = Value::from(build_discovery_micros);

    let relation_graph = facts
        .get("declarationRelations")
        .context("Kotlin facts have no declarationRelations")?;
    let descriptor_graph = facts
        .get("declarationDescriptors")
        .context("Kotlin facts have no declarationDescriptors")?;
    let provenance = relation_graph
        .get("provenance")
        .context("Kotlin relation graph has no provenance")?;

    let mut entities = BTreeMap::<String, Value>::new();
    for descriptor in descriptor_graph
        .get("descriptors")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let Some(symbol) = descriptor.get("symbolIdentity").and_then(Value::as_str) else {
            continue;
        };
        let range = range_validator.range_from_provider(descriptor, true)?;
        entities.insert(
            symbol.to_owned(),
            json!({
                "adapterNamespace":"codeclew.kotlin-k2/0.1",
                "opaqueId":symbol,
                "resolution":if descriptor.get("resolution").and_then(Value::as_str) == Some("PROVEN") {"RESOLVED"} else {"UNRESOLVED"},
                "coarseKind":coarse_kind(descriptor.get("declarationKind").and_then(Value::as_str)),
                "displayName":descriptor.get("compilerCallableId").or_else(|| descriptor.get("compilerClassId")).cloned().unwrap_or_else(|| Value::String(symbol.to_owned())),
                "primaryDefinition":range,
                "languagePayload":descriptor,
            }),
        );
    }
    let mut partial_core_index = PartialCoreIndex::default();
    for entity in entities.values() {
        if let Some(descriptor) = entity.get("languagePayload") {
            partial_core_index.index_emitted_descriptor(descriptor)?;
        }
    }

    let graph_coverage = relation_graph
        .get("coverage")
        .and_then(Value::as_str)
        .unwrap_or("UNKNOWN");
    let relation_enumeration_complete =
        graph_coverage == "COMPLETE_SUPPORTED_SUBSET" && build_model_boundaries.is_empty();
    let endpoint_identities = unique_descriptor_endpoint_identities(descriptor_graph);
    let mut descriptor_kind_candidates = BTreeMap::<String, BTreeSet<String>>::new();
    for descriptor in descriptor_graph
        .get("descriptors")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        if let (Some(symbol), Some(kind)) = (
            descriptor.get("symbolIdentity").and_then(Value::as_str),
            descriptor.get("declarationKind").and_then(Value::as_str),
        ) {
            descriptor_kind_candidates
                .entry(symbol.to_owned())
                .or_default()
                .insert(kind.to_owned());
        }
    }
    let descriptor_kinds = descriptor_kind_candidates
        .into_iter()
        .filter_map(|(symbol, mut kinds)| {
            (kinds.len() == 1).then(|| (symbol, kinds.pop_first().expect("one kind")))
        })
        .collect::<BTreeMap<_, _>>();
    let mut unresolved_endpoints =
        BTreeMap::<(String, String, BoundaryTargetBucket), BoundaryScopeAggregate>::new();
    let mut relation_facts = Vec::new();
    let mut relation_fact_ids = BTreeSet::new();
    let mut relation_fact_topology = BTreeSet::new();
    let mut retained_relation_witnesses =
        BTreeMap::<SourceOccurrenceKey, Vec<RetainedRelationWitness>>::new();
    let mut topology_owners = ExactCallTopologyOwnerIndex::default();
    for boundary in relation_graph
        .get("boundaries")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        topology_owners.index_quarantine_boundary(boundary)?;
    }
    let mut relation_kinds = BTreeSet::new();
    for relation in relation_graph
        .get("relations")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let kind = relation
            .get("kind")
            .and_then(Value::as_str)
            .unwrap_or("UNKNOWN");
        relation_kinds.insert(kind.to_owned());
        let raw_owner = relation
            .get("owner")
            .and_then(Value::as_str)
            .unwrap_or("<unknown-owner>");
        let raw_target_value = relation.get("target").and_then(Value::as_str);
        let raw_target = raw_target_value.unwrap_or("<unknown-target>");
        let (target_scope, target_query_key) = boundary_target_scope(raw_target_value)?;
        let resolved_owner = endpoint_identities.get(raw_owner);
        let target_needs_identity = raw_target != "null" && !raw_target.starts_with('<');
        let resolved_target = target_needs_identity
            .then(|| endpoint_identities.get(raw_target))
            .flatten();
        let range = range_validator.range_from_provider(relation, true)?;
        topology_owners.index_verified_relation(relation)?;
        partial_core_index.index_verified_relation_core(relation)?;
        let endpoint_coverage = RelationEndpointCoverage::new(
            raw_target,
            resolved_owner.is_some(),
            resolved_target.is_some(),
        );
        for (role, raw) in [("OWNER", raw_owner), ("TARGET", raw_target)] {
            if endpoint_coverage.role_missing(role) {
                let row_hash = canonical_hash(&json!({"raw":raw,"relation":relation}))?;
                let owner_query_key = query_key_for_raw_endpoint(raw_owner)?;
                let target_bucket = boundary_target_bucket(target_scope, target_query_key.clone());
                let group = unresolved_endpoints
                    .entry((role.to_owned(), kind.to_owned(), target_bucket.clone()))
                    .or_default();
                if group.row_hashes.insert(row_hash) {
                    group.affected_row_count = group.affected_row_count.saturating_add(1);
                    if let Some(key) = owner_query_key {
                        group.owner_query_keys.insert(key);
                    }
                    if let Some(key) = &target_bucket.target_query_key {
                        group.target_query_keys.insert(key.clone());
                    } else if target_bucket.scope == BoundaryTargetScope::Global {
                        group.has_global_target = true;
                    }
                }
            }
        }
        if !endpoint_coverage.endpoints_resolved() {
            continue;
        }
        let owner = resolved_owner.expect("resolved owner").as_str();
        let target = resolved_target.map(String::as_str).unwrap_or(raw_target);
        let operation = format!("codeclew.relation/{}/1", kind.to_ascii_lowercase());
        let range_hash = canonical_hash(&range)?;
        relation_fact_topology.insert((
            operation.clone(),
            owner.to_owned(),
            target.to_owned(),
            range_hash,
        ));
        if relation.get("schema").and_then(Value::as_str) == Some("declaration-relation/0.1")
            && relation.get("resolution").and_then(Value::as_str) == Some("PROVEN")
            && relation.get("provider").and_then(Value::as_str) == Some("K2_FIR")
            && let Some(occurrence) = source_occurrence_key(relation)?
            && let Some(target_query_key) = query_key_for_raw_endpoint(raw_target)?
        {
            retained_relation_witnesses
                .entry(occurrence)
                .or_default()
                .push(RetainedRelationWitness {
                    kind: kind.to_owned(),
                    operation: operation.clone(),
                    raw_hash: canonical_hash(relation)?,
                    target_query_key,
                    resolved_owner: owner.to_owned(),
                    resolved_target: target.to_owned(),
                    target_declaration_kind: descriptor_kinds.get(target).cloned(),
                    range: range.clone(),
                });
        }
        let fact_id = canonical_hash(&json!({
            "provider":"K2_FIR",
            "kind":kind,
            "owner":owner,
            "target":target,
            "range":range,
            "snapshot":repository_tree_digest,
        }))?;
        if !relation_fact_ids.insert(fact_id.clone()) {
            continue;
        }
        relation_facts.push(json!({
            "factId":fact_id,
            "relation":operation,
            "owner":owner,
            "target":target,
            "truth":"TRUE",
            "grade":"COMPILER_RESOLVED",
            "enumeration":if relation_enumeration_complete {"COMPLETE_IN_SCOPE"} else {"PARTIAL"},
            "range":range,
            "providerPayload":relation,
        }));
    }

    let mut boundaries = build_model_boundaries;
    for ((role, kind, target_bucket), group) in unresolved_endpoints {
        let effect = if target_bucket.scope == BoundaryTargetScope::OutOfScope {
            BOUNDARY_EFFECT_OUT_OF_SCOPE
        } else {
            BOUNDARY_EFFECT_TOPOLOGY
        };
        let exact_target_scope = target_bucket.scope == BoundaryTargetScope::Exact;
        let applicability = boundary_applicability(
            effect,
            [format!("codeclew.relation/{}/1", kind.to_ascii_lowercase())],
            &group.owner_query_keys,
            &group.target_query_keys,
            exact_target_scope,
        );
        boundaries.push(json!({
            "boundaryId":canonical_hash(&json!({
                "kind":"unresolved-relation-endpoint",
                "role":role,
                "relationKind":kind,
                "rowHashes":group.row_hashes,
                "applicability":applicability,
            }))?,
            "kindUri":"codeclew.boundary/kotlin/unresolved-relation-endpoint/1",
            "consequence":"ENUMERATION_INCOMPLETE",
            "origin":Value::Null,
            "provider":"K2_FIR",
            "applicability":applicability,
            "details":{
                "role":role,
                "relationKind":kind,
                "affectedRowCount":group.affected_row_count,
                "rawRowsHash":canonical_hash(&json!(group.row_hashes))?,
            },
        }));
    }
    let mut compiler_boundary_groups = BTreeMap::<
        (
            String,
            String,
            String,
            String,
            String,
            BoundaryTargetBucket,
            BTreeSet<String>,
        ),
        BoundaryScopeAggregate,
    >::new();
    for (domain, graph) in [
        ("relation", relation_graph),
        ("descriptor", descriptor_graph),
    ] {
        for boundary in graph
            .get("boundaries")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let provider = boundary
                .get("provider")
                .and_then(Value::as_str)
                .unwrap_or("UNKNOWN");
            let stage = boundary
                .get("stage")
                .and_then(Value::as_str)
                .unwrap_or("UNKNOWN");
            let code = boundary
                .get("code")
                .and_then(Value::as_str)
                .unwrap_or("UNKNOWN");
            let boundary_hash = canonical_hash(boundary)?;
            let affected_rows = boundary
                .get("affectedRowCount")
                .or_else(|| boundary.pointer("/details/affectedRowCount"))
                .and_then(Value::as_u64)
                .unwrap_or(1);
            let mut proof = compiler_boundary_relation_proof(
                domain,
                provider,
                stage,
                code,
                boundary,
                &topology_owners,
                &retained_relation_witnesses,
                &repository_tree_digest,
            )?;
            let partial_core_pairing =
                partial_core_index.boundary_pairing(domain, provider, stage, code, boundary);
            apply_partial_core_pairing(&mut proof, partial_core_pairing);
            if let Some(fact) = proof.derived_fact.take() {
                let relation = fact
                    .get("relation")
                    .and_then(Value::as_str)
                    .context("derived relation fact has no operation")?;
                let owner = fact
                    .get("owner")
                    .and_then(Value::as_str)
                    .context("derived relation fact has no owner")?;
                let target = fact
                    .get("target")
                    .and_then(Value::as_str)
                    .context("derived relation fact has no target")?;
                let range_hash = canonical_hash(
                    fact.get("range")
                        .context("derived relation fact has no source range")?,
                )?;
                if relation_fact_topology.insert((
                    relation.to_owned(),
                    owner.to_owned(),
                    target.to_owned(),
                    range_hash,
                )) {
                    let fact_id = fact
                        .get("factId")
                        .and_then(Value::as_str)
                        .context("derived relation fact has no factId")?;
                    if !relation_fact_ids.insert(fact_id.to_owned()) {
                        anyhow::bail!(
                            "derived relation factId collides with retained compiler fact"
                        );
                    }
                    if relation == "codeclew.relation/calls/1" {
                        relation_kinds.insert("CALLS".to_owned());
                    } else if relation == "codeclew.relation/constructs/1" {
                        relation_kinds.insert("CONSTRUCTS".to_owned());
                    }
                    relation_facts.push(fact);
                }
            }
            let effect = compiler_boundary_effect_with_partial_core(
                domain,
                provider,
                stage,
                code,
                !proof.retained_base_operations.is_empty(),
                proof.source_boundary_valid,
                proof.target_scope,
                partial_core_pairing,
            );
            let mut operations = compiler_boundary_operations_for_proof(stage, code, &proof);
            if provider == "K2_FIR_CFG"
                && stage == "ORDER_PROVENANCE"
                && code == "NO_CFG_NODE_FOR_RELATION"
            {
                operations.extend(proof.retained_base_operations.iter().cloned());
            }
            let target_bucket =
                boundary_target_bucket(proof.target_scope, proof.target_query_key.clone());
            let group = compiler_boundary_groups
                .entry((
                    domain.to_owned(),
                    provider.to_owned(),
                    stage.to_owned(),
                    code.to_owned(),
                    effect.to_owned(),
                    target_bucket.clone(),
                    operations,
                ))
                .or_default();
            if group.row_hashes.insert(boundary_hash) {
                group.affected_row_count = group.affected_row_count.saturating_add(affected_rows);
                if let Some(key) = proof.owner_query_key {
                    group.owner_query_keys.insert(key);
                }
                if target_bucket.scope == BoundaryTargetScope::Exact
                    && let Some(key) = target_bucket.target_query_key
                {
                    group.target_query_keys.insert(key);
                } else {
                    group.has_global_target = true;
                }
            }
        }
    }
    for ((domain, provider, stage, code, effect, target_bucket, operations), group) in
        compiler_boundary_groups
    {
        let exact_target_scope = target_bucket.scope == BoundaryTargetScope::Exact
            && !group.has_global_target
            && !group.target_query_keys.is_empty();
        let applicability = boundary_applicability(
            &effect,
            operations,
            &group.owner_query_keys,
            &group.target_query_keys,
            exact_target_scope,
        );
        boundaries.push(json!({
            "boundaryId":canonical_hash(&json!({
                "domain":domain,
                "provider":provider,
                "stage":stage,
                "code":code,
                "sourceBoundaryHashes":group.row_hashes,
                "applicability":applicability,
            }))?,
            "kindUri":format!("codeclew.boundary/kotlin/{}/1",code.to_ascii_lowercase().replace('_',"-")),
            "consequence":"ENUMERATION_INCOMPLETE",
            "origin":Value::Null,
            "provider":"K2_FIR",
            "applicability":applicability,
            "details":{
                "domain":domain,
                "sourceProvider":provider,
                "stage":stage,
                "code":code,
                "affectedBoundaryCount":group.row_hashes.len(),
                "affectedRowCount":group.affected_row_count,
                "sourceBoundariesHash":canonical_hash(&json!(group.row_hashes))?,
            },
        }));
    }
    boundaries.sort_by(|left, right| {
        left["boundaryId"]
            .as_str()
            .cmp(&right["boundaryId"].as_str())
    });
    boundaries.dedup_by(|left, right| left["boundaryId"] == right["boundaryId"]);
    let complete_in_scope = graph_coverage == "COMPLETE_SUPPORTED_SUBSET" && boundaries.is_empty();
    if !complete_in_scope {
        for fact in &mut relation_facts {
            fact["enumeration"] = Value::String("PARTIAL".to_owned());
        }
    }

    let known_boundary_kinds = boundaries
        .iter()
        .filter_map(|value| value.get("kindUri").and_then(Value::as_str))
        .map(ToOwned::to_owned)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let mut capability_descriptors = Vec::new();
    for kind in relation_kinds {
        let operation_uri = format!("codeclew.relation/{}/1", kind.to_ascii_lowercase());
        capability_descriptors.push(json!({
                "operationUri":operation_uri,
                "operationVersion":"1",
                "operationSpecificationDigest":canonical_hash(&json!({"uri":operation_uri,"version":"1"}))?,
                "languageId":"kotlin",
                "adapterId":"codeclew.kotlin-k2",
                "adapterVersion":"0.1.0",
                "toolchainDigest":provenance.get("pluginArtifactFingerprint"),
                "buildConfigurationDigest":provenance.get("compilerOptionsHash"),
                "targetDigest":provenance.get("compilerOptionsHash"),
                "grade":"COMPILER_RESOLVED",
                "support":"SUPPORTED",
                "guaranteedEnumeration":if complete_in_scope {"COMPLETE_IN_SCOPE"} else {"PARTIAL"},
                "approximation":"SOUND_UNDER",
                "knownBoundaryKinds":known_boundary_kinds,
                "costClass":"codeclew.cost/compiler-frontend/1",
            }));
    }

    let entity_values = entities.into_values().collect::<Vec<_>>();
    context.enter(
        "DETERMINISTIC_SEED_SELECTION",
        "PARTIAL",
        "NO_ELIGIBLE_SEED",
    );
    let (seed, selection_authority) = if let Some(requested) = args.seed_entity.as_deref() {
        (
            resolve_requested_seed(requested, &entity_values, &relation_facts)?,
            "CALLER_SELECTED_RESOLVED_ENTITY",
        )
    } else {
        (
            derive_seed(&entity_values, &relation_facts)?,
            "DETERMINISTIC_LEXICOGRAPHIC_CANDIDATE",
        )
    };
    context.selected_inputs["query"]["proposedSeedEntity"] = Value::String(seed.clone());
    context.selected_inputs["query"]["selectionAuthority"] =
        Value::String(selection_authority.to_owned());
    let impact_start = Instant::now();
    let mut impact = deterministic_impact(
        &seed,
        &entity_values,
        &relation_facts,
        &boundaries,
        args.max_depth,
        args.max_entities,
        selection_authority,
    )?;
    let query_micros = impact_start.elapsed().as_micros() as u64;
    impact["queryMicros"] = Value::from(query_micros);
    context.telemetry.query_projection_micros = query_micros;

    context.enter("WORKER_SHUTDOWN", "FAILED", "WORKER_SHUTDOWN_FAILED");
    worker.shutdown()?;
    context.enter(
        "SOURCE_MUTATION_CHECK",
        "FAILED",
        "SOURCE_MUTATION_DETECTED",
    );
    verify_repository_unchanged(
        &repo,
        &ignored_components,
        &project,
        &sources,
        &repository_tree_digest,
        initial_git_status_digest.as_deref(),
        &generated_boundaries,
        &tracked_symlinks,
    )?;

    let compiler_version = required_string(provenance, "/compilerVersion")?;
    let adapter = AdapterIdentity {
        adapter_id: "codeclew.kotlin-k2".to_owned(),
        version: "0.1.0".to_owned(),
        binary_digest: adapter_binary.clone(),
        language_id: "kotlin".to_owned(),
    };
    let build_configuration_digest =
        required_string(provenance, "/compilerOptionsHash")?.to_owned();
    let dependency_graph_digest = canonical_hash(&json!({
        "orderedCompileClasspath":semantic_input_manifest.get("orderedCompileClasspath"),
        "orderedFriendPaths":semantic_input_manifest.get("orderedFriendPaths"),
        "orderedCompilerPlugins":semantic_input_manifest.get("orderedCompilerPlugins"),
        "dependencyCoordinates":semantic_input_manifest.get("dependencyCoordinates"),
        "repositories":semantic_input_manifest.get("repositories"),
        "reactorPoms":semantic_input_manifest.get("reactorPoms"),
        "buildPlugins":semantic_input_manifest.get("buildPlugins"),
        "generatedSourceConfiguration":semantic_input_manifest.get("generatedSourceConfiguration"),
        "fieldBoundaries":semantic_input_manifest.get("fieldBoundaries"),
        "buildModelBoundaries":semantic_input_manifest.get("buildModelBoundaries"),
        "legacyClasspathHash":provenance.get("classpathHash"),
    }))?;
    let generated_sources_manifest_digest = canonical_hash(&json!({
        "semanticInputManifestHash":semantic_input_manifest_hash,
        "configuration":semantic_input_manifest.get("generatedSourceConfiguration"),
        "sources":sources.iter().filter(|source| source.origin == "GENERATED").collect::<Vec<_>>(),
        "generatedBoundaries":boundaries.iter().filter(|boundary| {
            boundary.get("kindUri").and_then(Value::as_str).is_some_and(|kind| kind.contains("generated"))
                || boundary.pointer("/details/code").and_then(Value::as_str).is_some_and(|code| code.contains("GENERATED"))
        }).collect::<Vec<_>>()
    }))?;
    let stored_fact_bytes = canonical_bytes(&relation_facts)?.len() as u64;
    let adapter_micros = adapter_start.elapsed().as_micros() as u64;
    let mut output = AdapterOutput {
        schema: ADAPTER_OUTPUT_SCHEMA.to_owned(),
        adapter,
        snapshot_input: SnapshotInput {
            repository_tree_digest: repository_tree_digest.clone(),
            vcs_revision: vcs_revision.clone(),
            dirty,
            sources,
            build_system_uri: format!(
                "codeclew.build-system/{}/1",
                project
                    .get("buildSystem")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown")
                    .to_ascii_lowercase()
            ),
            build_model_digest: build_model_digest.clone(),
            build_configuration_digest: build_configuration_digest.clone(),
            dependency_graph_digest,
            toolchain: json!({
                "toolUri":"codeclew.toolchain/kotlin-k2/1",
                "version":compiler_version,
                "distributionDigest":provenance.get("pluginArtifactFingerprint"),
                "providerPayload":{
                    "declaredProjectCompilerVersion":semantic_input_manifest.get("declaredCompilerVersion"),
                    "analyzerCompilerVersion":project.get("workerCompilerVersion"),
                    "workerVersion":provenance.get("workerVersion"),
                    "workerProtocolVersion":provenance.get("workerProtocolVersion"),
                    "trustedDistributionTreeHash":context.provenance.pointer("/trustedWorkerDistribution/treeHash"),
                    "trustedDistributionBuildInputDigest":context.provenance.pointer("/trustedWorkerDistribution/buildInputDigest"),
                    "trustedDistributionPluginFingerprint":context.provenance.pointer("/trustedWorkerDistribution/pluginFingerprint"),
                },
            }),
            targets: vec![json!({
                "targetId":args.compilation,
                "configurationDigest":build_configuration_digest,
                "enabledFeatures":[],
                "platform":"JVM",
                "compilerFlags":project.get("freeCompilerArguments").cloned().unwrap_or_else(|| json!([])),
                "providerPayload":{
                    "compilerFlagsDigest":provenance.get("compilerOptionsHash"),
                },
            })],
            relevant_environment: vec![
                json!({"key":"semanticInputManifestHash","value":semantic_input_manifest_hash}),
                json!({"key":"declaredCompilerVersion","value":semantic_input_manifest.get("declaredCompilerVersion").and_then(Value::as_str).unwrap_or("UNKNOWN")}),
                json!({"key":"buildModelBoundarySetDigest","value":canonical_hash(&context.boundaries)?}),
                json!({"key":"trackedSymlinkManifestDigest","value":tracked_symlink_manifest_digest}),
            ],
            generated_sources_manifest_digest,
        },
        capability_descriptors,
        entities: entity_values,
        // Impact uses source-bound entities and facts directly. Materializing
        // and individually sealing a duplicate occurrence for every entity
        // and relation made real projects spend minutes after K2 completed.
        occurrences: Vec::new(),
        facts: relation_facts,
        boundaries,
        compiler_receipt: json!({
            "schema":"codeclew.compiler-receipt/0.1",
            "method":"K2_FIR_ANALYSIS",
            "status":"ACCEPTED",
            "grade":"COMPILER_CHECKED",
            "snapshotTreeDigest":repository_tree_digest,
            "claim":"Compiler frontend accepted and extracted the selected compilation; no behavioral-equivalence claim is made",
            "providerPayload":{
                "projectModelDigest":build_model_digest,
                "compilerVersion":compiler_version,
                "declaredCompilerVersion":semantic_input_manifest.get("declaredCompilerVersion"),
                "analyzerCompilerVersion":project.get("workerCompilerVersion"),
                "adapterBinaryDigest":adapter_binary,
                "semanticInputManifestHash":semantic_input_manifest_hash,
                "semanticInputManifest":semantic_input_manifest,
                "trustedWorkerDistribution":context.provenance.get("trustedWorkerDistribution"),
                "k2Validated":true,
            },
        }),
        impact,
        cost: CostRecord {
            total_wall_micros: total_start.elapsed().as_micros() as u64,
            repository_snapshot_micros: snapshot_micros,
            build_discovery_micros,
            cold_index_micros: index_micros,
            warm_index_micros: 0,
            adapter_micros,
            query_micros,
            source_bytes_read,
            emitted_bytes: 0,
            stored_fact_bytes,
            model_visible_source_bytes: 0,
            cache_requests: build_profile.cache_requests
                + profile.cache_requests
                + u64::from(semantic_cache.is_some()),
            cache_hits: build_profile.cache_hits + profile.cache_hits,
            provider_processing_micros: context.telemetry.provider_processing_micros,
        },
        output_digest: String::new(),
    };
    validate_adapter_ranges(&output, &range_validator)?;
    output.cost.total_wall_micros = total_start.elapsed().as_micros() as u64;
    output.seal()?;
    context.enter(
        "EVIDENCE_CORE_VALIDATION",
        "FAILED",
        "EVIDENCE_CORE_VALIDATION_FAILED",
    );
    let core = kotlin_k1::validate_kotlin_core_binding(&output)?;
    context.telemetry.stored_fact_bytes = stored_fact_bytes;
    context.telemetry.fact_count = output.facts.len() as u64;
    context.telemetry.boundary_count = output.boundaries.len() as u64;
    context.boundaries = output.boundaries.clone();

    if let Some(cache) = &semantic_cache {
        context.enter("CACHE_PUBLICATION", "FAILED", "CACHE_PUBLICATION_FAILED");
        let publication = cache.publish(&cache_key, &output, &core)?;
        context.telemetry.cache_bytes_written = publication.bytes_written;
        context.telemetry.store_write_micros = publication.write_micros;
        context.cache["status"] = Value::String("PUBLISHED_COLD".to_owned());
        context.cache["hit"] = Value::Bool(false);
        context.cache["semanticOutputDigest"] =
            Value::String(kotlin_k1::semantic_output_digest(&output)?);
        context.cache["semanticFactsDigest"] =
            Value::String(kotlin_k1::semantic_facts_digest(&output)?);
    }
    context.telemetry.cache_requests = output.cost.cache_requests;
    context.telemetry.cache_hits = output.cost.cache_hits;
    context.enter("COMPLETE", "FAILED", "UNREACHABLE_COMPLETE_FAILURE");
    Ok(RunSuccess {
        output: Some(output),
        agent_projection: None,
        core: Some(core),
        agent_output: args.agent_output,
    })
}

fn validate_run_phase(phase: RunPhase, seed: Option<&str>, has_cache: bool) -> Result<()> {
    match (phase, seed, has_cache) {
        (RunPhase::Warm, None, _) => {
            anyhow::bail!("warm projection run requires an explicit seed")
        }
        (RunPhase::Warm, Some(_), false) => {
            anyhow::bail!("warm projection run requires --state-root for a verified cache hit")
        }
        _ => Ok(()),
    }
}

fn validate_prepared_refusal_cli(args: &Args) -> Result<bool> {
    let present = [
        args.prepared_refusal.is_some(),
        args.prepared_refusal_sha256.is_some(),
        args.entry_id.is_some(),
        args.candidate_tools_sha256.is_some(),
        args.build_input_digest.is_some(),
        args.preparation_receipt_digest.is_some(),
    ];
    if present.iter().all(|value| !value) {
        return Ok(false);
    }
    if !present.iter().all(|value| *value) {
        anyhow::bail!("prepared refusal requires its complete exact binding argument set");
    }
    if args.build_state_root.is_some() {
        anyhow::bail!("prepared refusal must not consume an external build-state root");
    }
    Ok(true)
}

fn validate_prepared_refusal_phase(phase: RunPhase, seed: Option<&str>) -> Result<()> {
    if !matches!(phase, RunPhase::Cold) || seed.is_some() {
        anyhow::bail!("prepared refusal is accepted only as an independent COLD/no-seed run");
    }
    Ok(())
}

fn consume_prepared_refusal(args: &Args, repo: &Path, context: &mut RunContext) -> Result<()> {
    context.enter(
        "DEPENDENCY_PREPARATION_AUTHORITY",
        "FAILED",
        "PREPARED_REFUSAL_INVALID",
    );
    let path = args
        .prepared_refusal
        .as_deref()
        .context("prepared refusal path is absent")?;
    let file_digest = args
        .prepared_refusal_sha256
        .as_deref()
        .context("prepared refusal file digest is absent")?;
    let refusal = read_prepared_refusal(path, repo, file_digest)?;
    bind_prepared_refusal_cli(args, &refusal)?;
    validate_refusal_entry_cohort(&refusal)?;
    if detect_repository_build_dsl(repo)? != refusal.build_dsl {
        anyhow::bail!("prepared refusal buildDsl differs from the local repository");
    }

    let source_start = Instant::now();
    let observation_before = git_source_observation(repo)?;
    if observation_before.head != refusal.commit
        || observation_before.tree != refusal.git_tree
        || observation_before.source_tree_sha256 != refusal.source_tree_sha256
        || !observation_before.clean
    {
        anyhow::bail!("prepared refusal differs from the exact local Git/source authority");
    }
    let observation_after = git_source_observation(repo)?;
    if observation_before != observation_after {
        anyhow::bail!("repository source authority changed while validating prepared refusal");
    }
    context.telemetry.source_hashing_micros = source_start.elapsed().as_micros() as u64;

    context.selected_inputs = json!({
        "compilation":args.compilation,
        "runPhase":"COLD",
        "query":{
            "requestedSeedEntity":Value::Null,
            "maxDepth":args.max_depth,
            "maxEntities":args.max_entities,
        },
        "semanticCacheRequested":args.state_root.is_some(),
        "externalBuildStateRequested":false,
        "preparedRefusalRequested":true,
        "preparedRefusal":{
            "schema":refusal.schema,
            "seriesId":refusal.series_id,
            "cohort":refusal.cohort,
            "entry":refusal.entry,
            "selectedCompilation":refusal.selected_compilation,
            "objectDigest":refusal.object_digest,
            "fileDigest":file_digest,
            "sourceTreeSha256":refusal.source_tree_sha256,
            "candidateToolsSha256":refusal.candidate_tools_sha256,
            "buildInputDigest":refusal.build_input_digest,
            "preparationReceiptDigest":refusal.preparation_receipt_digest,
        },
    });
    context.snapshot = json!({
        "repositoryTreeDigest":refusal.source_tree_sha256,
        "sourceTreeSha256":refusal.source_tree_sha256,
        "vcsRevision":refusal.commit,
        "gitTree":refusal.git_tree,
        "dirty":false,
        "gitIndexDigest":observation_before.index_digest,
        "gitStatusDigest":observation_before.status_digest,
    });
    context.provenance = json!({
        "adapterId":"codeclew.kotlin-k2",
        "adapterVersion":"0.1.0",
        "terminalAuthority":{
            "schema":refusal.schema,
            "seriesId":refusal.series_id,
            "cohort":refusal.cohort,
            "entry":refusal.entry,
            "buildDsl":refusal.build_dsl,
            "objectDigest":refusal.object_digest,
            "preparedRefusalSha256":file_digest,
            "safeDetailDigest":refusal.safe_detail_digest,
            "sandboxProfileSha256":refusal.sandbox_profile_sha256,
            "sourceTreeSha256":refusal.source_tree_sha256,
            "candidateToolsSha256":refusal.candidate_tools_sha256,
            "buildInputDigest":refusal.build_input_digest,
            "preparationReceiptDigest":refusal.preparation_receipt_digest,
            "preparationCost":refusal.cost,
        },
        "workerStarted":false,
        "modelCalls":0,
    });
    context.cache = json!({
        "status":"NOT_APPLICABLE_PREPARED_REFUSAL",
        "hit":false,
        "reason":"DEPENDENCY_PREPARATION_TERMINAL",
    });
    context.telemetry.source_bytes_read = observation_before.source_bytes;
    context.telemetry.dependency_preparation_micros = Value::from(refusal.cost.wall_micros);
    context.detail_digest_override = Some(refusal.safe_detail_digest.clone());
    context.enter(
        "DEPENDENCY_PREPARATION",
        "REFUSED",
        prepared_reason_code(&refusal.reason_code)?,
    );
    Ok(())
}

fn bind_prepared_refusal_cli(args: &Args, refusal: &PreparedRefusal) -> Result<()> {
    let exact = [
        (
            args.entry_id.as_deref(),
            refusal.entry.as_str(),
            "entry identity",
        ),
        (
            args.candidate_tools_sha256.as_deref(),
            refusal.candidate_tools_sha256.as_str(),
            "candidate-tools digest",
        ),
        (
            args.build_input_digest.as_deref(),
            refusal.build_input_digest.as_str(),
            "build-input digest",
        ),
        (
            args.preparation_receipt_digest.as_deref(),
            refusal.preparation_receipt_digest.as_str(),
            "preparation-receipt digest",
        ),
    ];
    for (actual, expected, label) in exact {
        if actual != Some(expected) {
            anyhow::bail!("prepared refusal {label} differs from the CLI authority");
        }
    }
    if args.compilation != refusal.selected_compilation {
        anyhow::bail!("prepared refusal compilation differs from --compilation");
    }
    Ok(())
}

fn prepared_reason_code(reason: &str) -> Result<&'static str> {
    match reason {
        "DEPENDENCY_CLOSURE_UNAVAILABLE" => Ok("DEPENDENCY_CLOSURE_UNAVAILABLE"),
        "OFFLINE_MODEL_PROBE_FAILED" => Ok("OFFLINE_MODEL_PROBE_FAILED"),
        "UNSUPPORTED_BUILD_CONFIGURATION" => Ok("UNSUPPORTED_BUILD_CONFIGURATION"),
        _ => anyhow::bail!("prepared refusal reason is outside the adapter whitelist"),
    }
}

fn validate_refusal_entry_cohort(refusal: &PreparedRefusal) -> Result<()> {
    let matches = match refusal.cohort.as_str() {
        "QUALIFICATION" => refusal.entry.starts_with("K1-Q"),
        "BLIND_HOLDOUT" => refusal.entry.starts_with("K1-H"),
        _ => false,
    };
    if !matches
        || refusal.entry.len() != 6
        || !refusal.entry[4..].bytes().all(|byte| byte.is_ascii_digit())
    {
        anyhow::bail!("prepared refusal entry/cohort identity mismatch");
    }
    Ok(())
}

fn detect_repository_build_dsl(repo: &Path) -> Result<&'static str> {
    let candidates = [
        ("settings.gradle.kts", "GRADLE_KOTLIN_DSL"),
        ("settings.gradle", "GRADLE_GROOVY_DSL"),
        ("pom.xml", "MAVEN"),
        ("build.gradle.kts", "GRADLE_KOTLIN_DSL"),
        ("build.gradle", "GRADLE_GROOVY_DSL"),
    ];
    let detected = candidates
        .into_iter()
        .filter_map(|(name, dsl)| {
            let path = repo.join(name);
            std::fs::symlink_metadata(path)
                .ok()
                .filter(|metadata| metadata.is_file() && !metadata.file_type().is_symlink())
                .map(|_| dsl)
        })
        .collect::<BTreeSet<_>>();
    if detected.len() != 1 {
        anyhow::bail!("repository root has no unique supported build DSL authority");
    }
    Ok(*detected.iter().next().expect("one detected build DSL"))
}

fn project_boundaries(project: &Value) -> Result<Vec<Value>> {
    let mut boundaries = Vec::new();
    let fields = project
        .get("fieldBoundaries")
        .and_then(Value::as_object)
        .context("OpenProject fieldBoundaries must be an object")?;
    for (field, reason) in fields {
        let available = reason
            .as_str()
            .is_some_and(|value| value.starts_with("AVAILABLE") || value.starts_with("COMPLETE"));
        if available || reason.is_null() {
            continue;
        }
        boundaries.push(json!({
            "boundaryId":canonical_hash(&json!({"domain":"build-model","field":field,"reason":reason}))?,
            "kindUri":format!("codeclew.boundary/kotlin-build-model/{}/1",field.to_ascii_lowercase().replace('_',"-")),
            "consequence":"ENUMERATION_INCOMPLETE",
            "origin":Value::Null,
            "provider":"KOTLIN_PROJECT_MODEL",
            "applicability":global_boundary_applicability(BOUNDARY_EFFECT_BUILD_FIDELITY),
            "details":{"field":field,"reason":reason},
        }));
    }
    for boundary in project
        .get("buildModelBoundaries")
        .and_then(Value::as_array)
        .context("OpenProject buildModelBoundaries must be an array")?
    {
        let code = boundary
            .as_str()
            .context("buildModelBoundary must be a string")?;
        boundaries.push(json!({
            "boundaryId":canonical_hash(&json!({"domain":"build-model","code":code}))?,
            "kindUri":format!("codeclew.boundary/kotlin-build-model/{}/1",uri_component(code)),
            "consequence":"ENUMERATION_INCOMPLETE",
            "origin":Value::Null,
            "provider":"KOTLIN_PROJECT_MODEL",
            "applicability":global_boundary_applicability(BOUNDARY_EFFECT_BUILD_FIDELITY),
            "details":{"code":code},
        }));
    }
    let generated_status = project
        .pointer("/generatedSourceConfiguration/status")
        .and_then(Value::as_str)
        .unwrap_or("UNAVAILABLE_PROVIDER");
    if !matches!(
        generated_status,
        "NONE_DISCOVERED" | "ROOTS_AND_BUILD_PLUGIN_SET"
    ) {
        boundaries.push(json!({
            "boundaryId":canonical_hash(&json!({"domain":"generated-source-configuration","status":generated_status}))?,
            "kindUri":"codeclew.boundary/kotlin-build-model/generated-source-configuration-incomplete/1",
            "consequence":"ENUMERATION_INCOMPLETE",
            "origin":Value::Null,
            "provider":"KOTLIN_PROJECT_MODEL",
            "applicability":global_boundary_applicability(BOUNDARY_EFFECT_BUILD_FIDELITY),
            "details":{"status":generated_status},
        }));
    }
    boundaries.sort_by(|left, right| {
        left["boundaryId"]
            .as_str()
            .cmp(&right["boundaryId"].as_str())
    });
    boundaries.dedup_by(|left, right| left["boundaryId"] == right["boundaryId"]);
    Ok(boundaries)
}

fn derive_seed(entities: &[Value], facts: &[Value]) -> Result<String> {
    let eligible = entities
        .iter()
        .filter(|entity| entity.get("resolution").and_then(Value::as_str) == Some("RESOLVED"))
        .filter(|entity| {
            entity
                .get("primaryDefinition")
                .is_some_and(|definition| !definition.is_null())
        })
        .filter_map(|entity| entity.get("opaqueId").and_then(Value::as_str))
        .filter(|candidate| {
            facts.iter().any(|fact| {
                fact.get("owner").and_then(Value::as_str) == Some(*candidate)
                    || fact.get("target").and_then(Value::as_str) == Some(*candidate)
            })
        })
        .collect::<BTreeSet<_>>();
    eligible
        .iter()
        .next()
        .map(|selected| (*selected).to_owned())
        .context("no resolved source-defined entity with an incident relation fact")
}

fn resolve_requested_seed(requested: &str, entities: &[Value], facts: &[Value]) -> Result<String> {
    let eligible = entities
        .iter()
        .filter(|entity| entity.get("resolution").and_then(Value::as_str) == Some("RESOLVED"))
        .filter(|entity| {
            entity
                .get("primaryDefinition")
                .is_some_and(|definition| !definition.is_null())
        })
        .collect::<Vec<_>>();
    let exact = eligible
        .iter()
        .filter(|entity| entity.get("opaqueId").and_then(Value::as_str) == Some(requested))
        .copied()
        .collect::<Vec<_>>();
    let matches = if exact.is_empty() {
        eligible
            .iter()
            .filter(|entity| {
                entity
                    .pointer("/languagePayload/compilerCallableId")
                    .and_then(Value::as_str)
                    == Some(requested)
                    || (is_class_like_descriptor(&entity["languagePayload"])
                        && entity
                            .pointer("/languagePayload/compilerClassId")
                            .and_then(Value::as_str)
                            == Some(requested))
            })
            .copied()
            .collect::<Vec<_>>()
    } else {
        exact
    };
    let [entity] = matches.as_slice() else {
        if matches.is_empty() {
            anyhow::bail!("requested seed does not resolve to a source-defined entity");
        }
        anyhow::bail!("requested seed is ambiguous across resolved source entities");
    };
    let selected = entity["opaqueId"]
        .as_str()
        .context("resolved seed has no opaqueId")?;
    if !facts.iter().any(|fact| {
        fact.get("owner").and_then(Value::as_str) == Some(selected)
            || fact.get("target").and_then(Value::as_str) == Some(selected)
    }) {
        anyhow::bail!("requested seed has no incident compiler relation");
    }
    Ok(selected.to_owned())
}

fn deterministic_impact(
    seed: &str,
    entities: &[Value],
    facts: &[Value],
    boundaries: &[Value],
    max_depth: usize,
    max_entities: usize,
    selection_authority: &str,
) -> Result<Value> {
    let mut impact = normalized_reverse_impact(
        Some(seed),
        entities,
        facts,
        boundaries,
        max_depth,
        max_entities,
    )?;
    impact["providerPayload"] = json!({
        "proposedSeedEntity":seed,
        "selectionAuthority":selection_authority,
    });
    Ok(impact)
}

fn unique_descriptor_endpoint_identities(descriptor_graph: &Value) -> BTreeMap<String, String> {
    let mut candidates = BTreeMap::<String, BTreeSet<String>>::new();
    for descriptor in descriptor_graph
        .get("descriptors")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let Some(symbol) = descriptor.get("symbolIdentity").and_then(Value::as_str) else {
            continue;
        };
        candidates
            .entry(symbol.to_owned())
            .or_default()
            .insert(symbol.to_owned());
        for field in ["compilerCallableId"] {
            if let Some(endpoint) = descriptor.get(field).and_then(Value::as_str) {
                candidates
                    .entry(endpoint.to_owned())
                    .or_default()
                    .insert(symbol.to_owned());
            }
        }
        if is_class_like_descriptor(descriptor)
            && let Some(endpoint) = descriptor.get("compilerClassId").and_then(Value::as_str)
        {
            candidates
                .entry(endpoint.to_owned())
                .or_default()
                .insert(symbol.to_owned());
        }
    }
    candidates
        .into_iter()
        .filter_map(|(endpoint, mut symbols)| {
            if symbols.len() != 1 {
                return None;
            }
            Some((endpoint, symbols.pop_first().expect("one symbol")))
        })
        .collect()
}

fn is_class_like_descriptor(descriptor: &Value) -> bool {
    matches!(
        descriptor.get("declarationKind").and_then(Value::as_str),
        Some("CLASS" | "INTERFACE" | "OBJECT" | "TYPE_ALIAS")
    )
}

fn normalized_reverse_impact(
    seed: Option<&str>,
    entities: &[Value],
    facts: &[Value],
    boundaries: &[Value],
    max_depth: usize,
    max_entities: usize,
) -> Result<Value> {
    let mut impact =
        bounded_reverse_impact(seed, entities, facts, boundaries, max_depth, max_entities)?;
    normalize_impact_boundary_obligations(&mut impact)?;
    Ok(impact)
}

fn normalize_impact_boundary_obligations(impact: &mut Value) -> Result<()> {
    let boundaries = impact
        .get("boundaries")
        .and_then(Value::as_array)
        .context("impact boundaries must be an array")?;
    let mut boundary_digests = BTreeSet::new();
    for boundary in boundaries {
        if !boundary.is_object() {
            anyhow::bail!("impact boundary must be an object");
        }
        let digest = canonical_hash(boundary)?;
        if !boundary_digests.insert(digest) {
            anyhow::bail!("impact contains duplicate canonical boundaries");
        }
    }
    let boundary_assessment = impact
        .get("boundaryAssessment")
        .context("impact boundary assessment is missing")?;
    let query_boundary_set_digest = canonical_hash(&boundaries)?;
    if boundary_assessment
        .get("queryRelevantBoundaryCount")
        .and_then(Value::as_u64)
        != Some(boundaries.len() as u64)
        || boundary_assessment
            .get("queryRelevantBoundarySetDigest")
            .and_then(Value::as_str)
            != Some(query_boundary_set_digest.as_str())
    {
        anyhow::bail!("impact boundary assessment differs from relevant boundary inventory");
    }
    let existing = impact
        .get("mandatoryObligations")
        .and_then(Value::as_array)
        .context("impact mandatoryObligations must be an array")?;
    let mut obligations = existing
        .iter()
        .filter(|obligation| {
            obligation.get("boundaryDigest").is_none()
                && obligation.get("kind").and_then(Value::as_str)
                    != Some("codeclew.obligation/impact-closure-completeness/1")
        })
        .cloned()
        .collect::<Vec<_>>();
    let mut path_fact_ids = impact
        .get("paths")
        .and_then(Value::as_array)
        .context("impact paths must be an array")?
        .iter()
        .filter_map(|path| path.get("factId").and_then(Value::as_str))
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();
    path_fact_ids.sort();
    path_fact_ids.dedup();
    let complete = boundaries.is_empty();
    let mut closure = json!({
        "id":"impact-closure-completeness",
        "kind":"codeclew.obligation/impact-closure-completeness/1",
        "mandatory":true,
        "status":if complete {"SATISFIED"} else {"UNKNOWN"},
        "evidenceFactIds":path_fact_ids,
        "providerPayload":{
            "boundaryAssessmentDigest":canonical_hash(boundary_assessment)?,
            "querySpecificationDigest":boundary_assessment.get("querySpecificationDigest"),
            "queryRelevantBoundaryCount":boundaries.len(),
            "queryRelevantBoundarySetDigest":canonical_hash(&boundaries)?,
        },
    });
    if !complete {
        closure["reason"] = Value::String("QUERY_TOPOLOGY_BOUNDARY_REMAINS".to_owned());
    }
    obligations.push(closure);
    impact["mandatoryObligations"] = Value::Array(obligations);
    Ok(())
}

fn validate_external_root(path: &Path, repo: &Path) -> Result<PathBuf> {
    if !path.is_absolute() {
        anyhow::bail!("external K1 state root must be absolute");
    }
    let metadata =
        std::fs::symlink_metadata(path).context("external K1 state root must already exist")?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        anyhow::bail!("external K1 state root must be a regular non-symlink directory");
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o022 != 0 {
            anyhow::bail!("external K1 state root must not be group- or world-writable");
        }
    }
    let root = path.canonicalize()?;
    if root != path {
        anyhow::bail!("external K1 state root must be canonical and have no symlinked ancestor");
    }
    let repo = repo.canonicalize()?;
    if root.starts_with(&repo) || repo.starts_with(&root) {
        anyhow::bail!("external K1 state root must not contain or be inside the repository");
    }
    Ok(root)
}

struct PreparedBuildStateIdentity {
    root: PathBuf,
    seed_digest: String,
    manifest_digest: String,
    marker_bytes_digest: String,
}

impl PreparedBuildStateIdentity {
    fn semantic_identity(&self) -> Value {
        json!({
            "mode":"EXTERNAL",
            "seedDigest":self.seed_digest,
            "manifestDigest":self.manifest_digest,
            "markerBytesDigest":self.marker_bytes_digest,
        })
    }
}

fn validate_build_state_root(path: &Path, repo: &Path) -> Result<PreparedBuildStateIdentity> {
    let root = validate_external_root(path, repo)?;
    for component in ["gradle-user-home", "maven-repository"] {
        let directory = root.join(component);
        let metadata = std::fs::symlink_metadata(&directory).with_context(|| {
            format!("external build state is not PREPARE-complete: {component} is absent")
        })?;
        if !metadata.is_dir()
            || metadata.file_type().is_symlink()
            || directory.canonicalize()? != directory
        {
            anyhow::bail!("external build-state component must be a contained real directory");
        }
    }
    let marker = root.join(BUILD_STATE_SEED_FILE);
    let metadata = std::fs::symlink_metadata(&marker).context(
        "external build state is not PREPARE-complete: CODECLEW_K1_BUILD_STATE_SEED is absent",
    )?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        anyhow::bail!("build-state seed must be a regular non-symlink file");
    }
    let manifest_path = root.join(BUILD_STATE_MANIFEST_FILE);
    let manifest_metadata = std::fs::symlink_metadata(&manifest_path)
        .context("external build state is not PREPARE-complete: canonical manifest is absent")?;
    if !manifest_metadata.is_file() || manifest_metadata.file_type().is_symlink() {
        anyhow::bail!("build-state manifest must be a regular non-symlink file");
    }
    if manifest_metadata.len() == 0 || manifest_metadata.len() > 64 * 1024 * 1024 {
        anyhow::bail!("build-state manifest size must be in 1..=67108864 bytes");
    }
    let manifest_bytes = std::fs::read(&manifest_path)?;
    let manifest: Value = serde_json::from_slice(&manifest_bytes)
        .context("external build-state manifest is invalid JSON")?;
    let manifest_object = manifest
        .as_object()
        .context("external build-state manifest must be an object")?;
    let expected_keys = [
        "schema",
        "seriesId",
        "cohort",
        "toolchain",
        "repositories",
        "gradleUserHomeTreeDigest",
        "mavenLocalRepositoryTreeDigest",
        "files",
        "seedDigest",
    ]
    .into_iter()
    .collect::<BTreeSet<_>>();
    if manifest_object
        .keys()
        .map(String::as_str)
        .collect::<BTreeSet<_>>()
        != expected_keys
        || manifest.get("schema").and_then(Value::as_str) != Some(BUILD_STATE_MANIFEST_SCHEMA)
        || !matches!(
            manifest.get("cohort").and_then(Value::as_str),
            Some("QUALIFICATION" | "BLIND_HOLDOUT" | "FIXTURE")
        )
        || manifest
            .get("seriesId")
            .and_then(Value::as_str)
            .is_none_or(str::is_empty)
        || !manifest.get("toolchain").is_some_and(Value::is_object)
        || !manifest.get("repositories").is_some_and(Value::is_array)
        || !manifest.get("files").is_some_and(Value::is_array)
    {
        anyhow::bail!("external build-state manifest schema differs from K1");
    }
    let canonical_with_newline = {
        let mut bytes = canonical_bytes(&manifest)?;
        bytes.push(b'\n');
        bytes
    };
    if manifest_bytes != canonical_with_newline {
        anyhow::bail!("external build-state manifest is not canonical JSON plus newline");
    }
    let seed_digest = required_string(&manifest, "/seedDigest")?.to_owned();
    for digest in [
        seed_digest.as_str(),
        required_string(&manifest, "/gradleUserHomeTreeDigest")?,
        required_string(&manifest, "/mavenLocalRepositoryTreeDigest")?,
    ] {
        if digest.len() != 71
            || !digest.starts_with("sha256:")
            || !digest[7..]
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            anyhow::bail!("external build-state manifest contains a malformed digest");
        }
    }
    let mut seed_body = manifest.clone();
    seed_body["seedDigest"] = Value::String(String::new());
    let mut seed_bytes = canonical_bytes(&seed_body)?;
    seed_bytes.push(b'\n');
    if seed_digest != hash_bytes(&seed_bytes) {
        anyhow::bail!("external build-state seedDigest is not self-consistent");
    }
    let manifest_digest = hash_bytes(&manifest_bytes);
    let marker_bytes = std::fs::read(&marker)?;
    if marker_bytes != format!("{manifest_digest}\n").into_bytes() {
        anyhow::bail!("external build-state marker does not seal the exact manifest");
    }
    Ok(PreparedBuildStateIdentity {
        root,
        seed_digest,
        manifest_digest,
        marker_bytes_digest: hash_bytes(&marker_bytes),
    })
}

fn validate_worker_build_state_identity(
    semantic_input_manifest: &Value,
    prepared: &PreparedBuildStateIdentity,
) -> Result<()> {
    let identity = semantic_input_manifest
        .get("buildState")
        .and_then(Value::as_object)
        .context("worker semantic manifest has no buildState identity")?;
    if identity.get("mode").and_then(Value::as_str) != Some("EXTERNAL")
        || identity.get("seedDigest").and_then(Value::as_str) != Some(prepared.seed_digest.as_str())
        || identity.get("manifestDigest").and_then(Value::as_str)
            != Some(prepared.manifest_digest.as_str())
        || identity.get("markerBytesDigest").and_then(Value::as_str)
            != Some(prepared.marker_bytes_digest.as_str())
        || identity.get("gradleUserHome").and_then(Value::as_str) != Some("gradle-user-home")
        || identity.get("mavenLocalRepository").and_then(Value::as_str) != Some("maven-repository")
        || identity.get("homeCredentials").and_then(Value::as_str) != Some("ISOLATED")
    {
        anyhow::bail!(
            "worker semantic manifest is not bound to the exact adapter-validated build state"
        );
    }
    let namespace = identity
        .get("namespaceDigest")
        .and_then(Value::as_str)
        .context("worker buildState identity has no namespaceDigest")?;
    if namespace.len() != 71
        || !namespace.starts_with("sha256:")
        || !namespace[7..]
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        anyhow::bail!("worker buildState namespaceDigest is malformed");
    }
    Ok(())
}

fn validate_repository_local_build_state_identity(semantic_input_manifest: &Value) -> Result<()> {
    let identity = semantic_input_manifest
        .get("buildState")
        .and_then(Value::as_object)
        .context("worker semantic manifest has no buildState identity")?;
    if identity.get("mode").and_then(Value::as_str) != Some("LEGACY_REPOSITORY_OWNED")
        || identity
            .get("seedDigest")
            .is_some_and(|value| !value.is_null())
        || identity
            .get("manifestDigest")
            .is_some_and(|value| !value.is_null())
        || identity.get("runtimeIsolation").and_then(Value::as_str)
            != Some("REPOSITORY_OWNED_LEGACY")
        || identity.get("gradleUserHome").and_then(Value::as_str) != Some("gradle-user-home")
        || identity.get("mavenLocalRepository").and_then(Value::as_str) != Some("maven-repository")
        || identity.get("homeCredentials").and_then(Value::as_str) != Some("INHERITED_LEGACY")
    {
        anyhow::bail!("worker did not retain explicit repository-local build-state identity");
    }
    let namespace = identity
        .get("namespaceDigest")
        .and_then(Value::as_str)
        .context("repository-local build state has no namespaceDigest")?;
    if namespace.len() != 71 || !namespace.starts_with("sha256:") {
        anyhow::bail!("repository-local build-state namespaceDigest is malformed");
    }
    Ok(())
}

/// Build tools often derive classpath-authority digests from absolute cache
/// paths. Those paths change when the same sealed dependency state is mounted
/// in a fresh private runtime, even though the ordered artifacts are exactly
/// the same. Cache and agent-output identity must describe the normalized
/// semantic classpath, not the disposable mount point.
fn stable_semantic_input_manifest(raw: &Value) -> Result<Value> {
    let mut stable = raw.clone();
    let classpath = stable
        .get("orderedCompileClasspath")
        .and_then(Value::as_array)
        .context("semantic input manifest has no orderedCompileClasspath")?;
    let mut ordered_bytes = Vec::new();
    for entry in classpath {
        let entry = entry
            .as_str()
            .context("semantic input classpath contains a non-string entry")?;
        ordered_bytes.extend_from_slice(entry.as_bytes());
        ordered_bytes.push(0);
    }
    let ordered_digest = Value::String(hash_bytes(&ordered_bytes));
    if let Some(authority) = stable
        .get_mut("classpathAuthority")
        .and_then(Value::as_object_mut)
    {
        if authority.contains_key("orderedDigest") {
            authority.insert("orderedDigest".to_owned(), ordered_digest.clone());
        }
        // Gradle records two raw-path authorities. They can be normalized to
        // the selected ordered classpath only when the provider proved them
        // equivalent; a real disagreement remains part of semantic identity.
        if authority.get("orderedEquivalent").and_then(Value::as_bool) == Some(true) {
            for field in ["taskLibrariesDigest", "configurationDigest"] {
                if authority.get(field).is_some_and(|value| !value.is_null()) {
                    authority.insert(field.to_owned(), ordered_digest.clone());
                }
            }
        }
    }
    Ok(stable)
}

fn stable_project_model_digest(
    project: &Value,
    semantic_input_manifest: &Value,
    semantic_input_manifest_hash: &str,
) -> Result<String> {
    let mut stable = project.clone();
    let object = stable
        .as_object_mut()
        .context("OpenProject response is not an object")?;
    object.remove("projectModelHash");
    object.insert(
        "semanticInputManifest".to_owned(),
        semantic_input_manifest.clone(),
    );
    object.insert(
        "semanticInputManifestHash".to_owned(),
        Value::String(semantic_input_manifest_hash.to_owned()),
    );
    if let Some(authority) = semantic_input_manifest.get("classpathAuthority") {
        object.insert("classpathAuthority".to_owned(), authority.clone());
    }
    canonical_hash(&stable)
}

fn ensure_external_roots_disjoint(left: &Path, right: &Path) -> Result<()> {
    if left.starts_with(right) || right.starts_with(left) {
        anyhow::bail!(
            "semantic cache and build state roots must be distinct non-overlapping directories"
        );
    }
    Ok(())
}

fn git_status_digest(repo: &Path) -> Result<Option<String>> {
    let head = sanitized_git_text(repo, &["rev-parse", "HEAD"])?;
    let tree = sanitized_git_text(repo, &["rev-parse", "HEAD^{tree}"])?;
    validate_git_object(&head)?;
    validate_git_object(&tree)?;
    Ok(Some(hash_clean_mismatches(&git_clean_mismatch_rows(
        repo, &tree,
    )?)?))
}

#[derive(Debug, PartialEq, Eq)]
struct GitSourceObservation {
    head: String,
    tree: String,
    clean: bool,
    status_digest: String,
    source_tree_sha256: String,
    index_digest: String,
    source_bytes: u64,
}

fn git_index_identity_rows(repo: &Path) -> Result<BTreeMap<String, Value>> {
    let output = sanitized_git_command(repo, &["ls-files", "-s", "-z", "--", "."])?;
    if !output.status.success() {
        anyhow::bail!("Git index identity observation failed");
    }
    let mut rows = BTreeMap::new();
    for raw in output
        .stdout
        .split(|byte| *byte == 0)
        .filter(|row| !row.is_empty())
    {
        let tab = raw
            .iter()
            .position(|byte| *byte == b'\t')
            .context("Git index identity row has no path separator")?;
        let header = std::str::from_utf8(&raw[..tab])?;
        let fields = header.split(' ').collect::<Vec<_>>();
        if fields.len() != 3
            || fields[2] != "0"
            || !matches!(fields[0], "100644" | "100755" | "120000")
        {
            anyhow::bail!("Git index identity contains an unsupported member");
        }
        validate_git_object(fields[1])?;
        let path = std::str::from_utf8(&raw[tab + 1..])?;
        if normalized_repository_path(Path::new(path))? != path || rows.contains_key(path) {
            anyhow::bail!("Git index identity path is unsafe or duplicated");
        }
        rows.insert(
            path.to_owned(),
            json!({"mode":fields[0],"gitObject":fields[1]}),
        );
    }
    Ok(rows)
}

fn validate_object_directory(repo: &Path) -> Result<PathBuf> {
    let git_dir = repo.join(".git");
    let objects = git_dir.join("objects");
    for (path, label) in [
        (&git_dir, "Git directory"),
        (&objects, "Git object directory"),
    ] {
        let metadata = std::fs::symlink_metadata(path)?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            anyhow::bail!("{label} must be a real directory");
        }
    }
    let alternates = objects.join("info/alternates");
    if std::fs::symlink_metadata(&alternates).is_ok() {
        anyhow::bail!("Git object directory alternates are forbidden");
    }
    for entry in WalkDir::new(&objects).follow_links(false) {
        let entry = entry?;
        let metadata = std::fs::symlink_metadata(entry.path())?;
        if metadata.file_type().is_symlink() || (!metadata.is_dir() && !metadata.is_file()) {
            anyhow::bail!("Git object directory contains a symlink or special object");
        }
    }
    Ok(objects.canonicalize()?)
}

fn filter_free_git_command(repo: &Path, arguments: &[&str]) -> Result<Output> {
    let objects = validate_object_directory(repo)?;
    let temporary = tempfile::Builder::new()
        .prefix("codeclew-k1-filter-free-git-")
        .tempdir_in("/tmp")?;
    let git_dir = temporary.path().join("git");
    std::fs::create_dir(&git_dir)?;
    std::fs::create_dir(git_dir.join("objects"))?;
    std::fs::create_dir(git_dir.join("refs"))?;
    std::fs::write(git_dir.join("HEAD"), b"ref: refs/heads/unborn\n")?;
    let output = Command::new("/usr/bin/git")
        .env_clear()
        .env("HOME", temporary.path())
        .env("PATH", "/usr/bin:/bin")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_COUNT", "0")
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_ATTR_NOSYSTEM", "1")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_ASKPASS", "/usr/bin/false")
        .env("SSH_ASKPASS", "/usr/bin/false")
        .env("GIT_PROTOCOL_FROM_USER", "0")
        .env("GIT_NO_REPLACE_OBJECTS", "1")
        .env("GIT_OBJECT_DIRECTORY", objects)
        .env("GIT_ALTERNATE_OBJECT_DIRECTORIES", "")
        .env("LANG", "C.UTF-8")
        .env("LC_ALL", "C.UTF-8")
        .arg(format!("--git-dir={}", git_dir.display()))
        .args(arguments)
        .output()?;
    if !output.status.success() {
        anyhow::bail!("filter-free Git command failed");
    }
    Ok(output)
}

fn filesystem_member_rows(
    root: &Path,
    exclusions: &RepositoryExclusions,
) -> Result<BTreeMap<String, Value>> {
    let mut rows = BTreeMap::new();
    for entry in WalkDir::new(root)
        .follow_links(false)
        .sort_by_file_name()
        .into_iter()
        .filter_entry(|entry| {
            entry
                .path()
                .strip_prefix(root)
                .is_ok_and(|relative| !exclusions.excludes(relative))
        })
    {
        let entry = entry?;
        let relative = entry.path().strip_prefix(root)?;
        if relative.as_os_str().is_empty() {
            continue;
        }
        let metadata = std::fs::symlink_metadata(entry.path())?;
        if metadata.is_dir() && !metadata.file_type().is_symlink() {
            continue;
        }
        let path = normalized_repository_path(relative)?;
        if path == ".git" || path.starts_with(".git/") {
            continue;
        }
        #[cfg(unix)]
        use std::os::unix::fs::PermissionsExt;
        let value = if metadata.file_type().is_symlink() {
            let target = std::fs::read_link(entry.path())?;
            let bytes = symlink_target_bytes(&target)?;
            json!({"kind":"SYMLINK","mode":"120000","size":bytes.len(),"sha256":hash_bytes(&bytes),"target":target.to_str().context("symlink target is not UTF-8")?})
        } else if metadata.is_file() {
            let bytes = std::fs::read(entry.path())?;
            #[cfg(unix)]
            let mode = if metadata.permissions().mode() & 0o111 != 0 {
                "100755"
            } else {
                "100644"
            };
            #[cfg(not(unix))]
            let mode = "100644";
            json!({"kind":"FILE","mode":mode,"size":bytes.len(),"sha256":hash_bytes(&bytes)})
        } else {
            json!({"kind":"SPECIAL"})
        };
        if rows.insert(path, value).is_some() {
            anyhow::bail!("filesystem observation contains a duplicate path");
        }
    }
    Ok(rows)
}

fn tar_octal(field: &[u8]) -> Result<usize> {
    let end = field
        .iter()
        .position(|byte| *byte == 0 || *byte == b' ')
        .unwrap_or(field.len());
    let text = std::str::from_utf8(&field[..end])?.trim();
    if text.is_empty() {
        return Ok(0);
    }
    usize::from_str_radix(text, 8).context("tar numeric field is not octal")
}

fn tar_string(field: &[u8]) -> Result<&str> {
    let end = field
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(field.len());
    Ok(std::str::from_utf8(&field[..end])?)
}

fn parse_git_ustar(raw: &[u8]) -> Result<BTreeMap<String, Value>> {
    let mut rows = BTreeMap::new();
    let mut cursor = 0_usize;
    let mut zero_blocks = 0_usize;
    while cursor + 512 <= raw.len() {
        let header = &raw[cursor..cursor + 512];
        cursor += 512;
        if header.iter().all(|byte| *byte == 0) {
            zero_blocks += 1;
            if zero_blocks == 2 {
                if raw[cursor..].iter().any(|byte| *byte != 0) {
                    anyhow::bail!("tar archive has nonzero trailing bytes");
                }
                return Ok(rows);
            }
            continue;
        }
        if zero_blocks != 0 {
            anyhow::bail!("tar archive contains an interior zero block");
        }
        if &header[257..263] != b"ustar\0" || &header[263..265] != b"00" {
            anyhow::bail!("Git archive is not exact POSIX ustar");
        }
        let expected_checksum = tar_octal(&header[148..156])?;
        let actual_checksum = header
            .iter()
            .enumerate()
            .map(|(index, byte)| {
                if (148..156).contains(&index) {
                    b' ' as usize
                } else {
                    *byte as usize
                }
            })
            .sum::<usize>();
        if expected_checksum != actual_checksum {
            anyhow::bail!("tar header checksum mismatch");
        }
        let name = tar_string(&header[..100])?;
        let prefix = tar_string(&header[345..500])?;
        let path = if prefix.is_empty() {
            name.to_owned()
        } else {
            format!("{prefix}/{name}")
        };
        let normalized = normalized_repository_path(Path::new(path.trim_end_matches('/')))?;
        if normalized != path.trim_end_matches('/') {
            anyhow::bail!("tar member path is unsafe or noncanonical");
        }
        let size = tar_octal(&header[124..136])?;
        let mode = tar_octal(&header[100..108])?;
        let padded = size.checked_add(511).context("tar member size overflow")? / 512 * 512;
        let data_end = cursor
            .checked_add(size)
            .context("tar member size overflow")?;
        let padded_end = cursor
            .checked_add(padded)
            .context("tar member size overflow")?;
        if padded_end > raw.len() || data_end > raw.len() {
            anyhow::bail!("tar member content is truncated");
        }
        match header[156] {
            0 | b'0' => {
                let content = &raw[cursor..data_end];
                let value = json!({"kind":"FILE","mode":if mode & 0o111 != 0 {"100755"} else {"100644"},"size":content.len(),"sha256":hash_bytes(content)});
                if rows.insert(normalized, value).is_some() {
                    anyhow::bail!("tar archive contains duplicate members");
                }
            }
            b'2' => {
                if size != 0 {
                    anyhow::bail!("tar symlink unexpectedly contains data");
                }
                let target = tar_string(&header[157..257])?;
                if target.is_empty() {
                    anyhow::bail!("tar symlink target is empty");
                }
                let bytes = target.as_bytes();
                let value = json!({"kind":"SYMLINK","mode":"120000","size":bytes.len(),"sha256":hash_bytes(bytes),"target":target});
                if rows.insert(normalized, value).is_some() {
                    anyhow::bail!("tar archive contains duplicate members");
                }
            }
            b'5' => {
                if size != 0 {
                    anyhow::bail!("tar directory unexpectedly contains data");
                }
            }
            _ => anyhow::bail!("tar archive contains an unsupported member type"),
        }
        cursor = padded_end;
    }
    anyhow::bail!("tar archive has no exact two-block terminator")
}

fn git_clean_mismatch_rows(repo: &Path, tree: &str) -> Result<Vec<Value>> {
    validate_git_object(tree)?;
    let index = git_index_identity_rows(repo)?;
    let tree_output = filter_free_git_command(repo, &["ls-tree", "-rz", "--full-tree", tree])?;
    let mut head = BTreeMap::new();
    for raw in tree_output
        .stdout
        .split(|byte| *byte == 0)
        .filter(|row| !row.is_empty())
    {
        let tab = raw
            .iter()
            .position(|byte| *byte == b'\t')
            .context("Git tree row has no path separator")?;
        let header = std::str::from_utf8(&raw[..tab])?
            .split(' ')
            .collect::<Vec<_>>();
        if header.len() != 3
            || header[1] != "blob"
            || !matches!(header[0], "100644" | "100755" | "120000")
        {
            anyhow::bail!("Git tree contains an unsupported member");
        }
        validate_git_object(header[2])?;
        let path = std::str::from_utf8(&raw[tab + 1..])?;
        if normalized_repository_path(Path::new(path))? != path
            || head
                .insert(
                    path.to_owned(),
                    json!({"mode":header[0],"gitObject":header[2]}),
                )
                .is_some()
        {
            anyhow::bail!("Git tree path is unsafe or duplicated");
        }
    }
    let mut mismatches = Vec::new();
    for path in head
        .keys()
        .chain(index.keys())
        .cloned()
        .collect::<BTreeSet<_>>()
    {
        if head.get(&path) != index.get(&path) {
            mismatches.push(json!({"path":path,"kind":"HEAD_INDEX","expected":head.get(&path),"actual":index.get(&path)}));
        }
    }
    let archive_parent = tempfile::Builder::new()
        .prefix("codeclew-k1-clean-observation-")
        .tempdir_in("/tmp")?;
    let archive = archive_parent.path().join("tree.tar");
    let archive_text = archive.to_str().context("archive path is not UTF-8")?;
    filter_free_git_command(
        repo,
        &[
            "-c",
            "core.attributesFile=/dev/null",
            "archive",
            "--format=tar",
            "--output",
            archive_text,
            tree,
        ],
    )?;
    let expected = parse_git_ustar(&std::fs::read(&archive)?)?;
    let ignored_components = [
        ".git",
        ".gradle",
        ".kotlin",
        ".semantic-thread",
        "build",
        "target",
        "node_modules",
        ".idea",
        ".vscode",
    ]
    .into_iter()
    .map(ToOwned::to_owned)
    .collect::<BTreeSet<_>>();
    let exclusions = repository_exclusions(repo, &ignored_components);
    let actual = filesystem_member_rows(repo, &exclusions)?;
    for path in expected
        .keys()
        .chain(actual.keys())
        .cloned()
        .collect::<BTreeSet<_>>()
    {
        if expected.get(&path) != actual.get(&path) {
            let kind = if index.contains_key(&path) {
                "WORKTREE"
            } else {
                "UNTRACKED"
            };
            mismatches.push(json!({"path":path,"kind":kind,"expected":expected.get(&path),"actual":actual.get(&path)}));
        }
    }
    mismatches.sort_by(|left, right| {
        (left["path"].as_str(), left["kind"].as_str())
            .cmp(&(right["path"].as_str(), right["kind"].as_str()))
    });
    Ok(mismatches)
}

fn hash_clean_mismatches(rows: &[Value]) -> Result<String> {
    let mut bytes = Vec::new();
    for row in rows {
        bytes.extend(canonical_bytes(row)?);
        bytes.push(b'\n');
    }
    Ok(hash_bytes(&bytes))
}

fn git_source_observation(repo: &Path) -> Result<GitSourceObservation> {
    let head = sanitized_git_text(repo, &["rev-parse", "HEAD"])?;
    let tree = sanitized_git_text(repo, &["rev-parse", "HEAD^{tree}"])?;
    validate_git_object(&head)?;
    validate_git_object(&tree)?;
    let mismatches = git_clean_mismatch_rows(repo, &tree)?;
    let (source_tree_sha256, index_digest, source_bytes) = match git_tracked_source_digest(repo) {
        Ok(value) => value,
        Err(_error) if !mismatches.is_empty() => {
            let dirty = json!({
                "schema":"codeclew.git-dirty-source/0.1",
                "mismatches":mismatches,
            });
            (canonical_hash_with_newline(&dirty)?, hash_bytes(b""), 0)
        }
        Err(error) => return Err(error),
    };
    Ok(GitSourceObservation {
        head,
        tree,
        clean: mismatches.is_empty(),
        status_digest: hash_clean_mismatches(&mismatches)?,
        source_tree_sha256,
        index_digest,
        source_bytes,
    })
}

fn sanitized_git_command(repo: &Path, arguments: &[&str]) -> Result<Output> {
    let output = Command::new("/usr/bin/git")
        .env_clear()
        .env("HOME", repo)
        .env("PATH", "/usr/bin:/bin")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_COUNT", "0")
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_ATTR_NOSYSTEM", "1")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_ASKPASS", "/usr/bin/false")
        .env("SSH_ASKPASS", "/usr/bin/false")
        .env("GIT_PROTOCOL_FROM_USER", "0")
        .env("GIT_NO_REPLACE_OBJECTS", "1")
        .env("LANG", "C.UTF-8")
        .env("LC_ALL", "C.UTF-8")
        .arg("-C")
        .arg(repo)
        .args(arguments)
        .output()?;
    Ok(output)
}

fn sanitized_git_text(repo: &Path, arguments: &[&str]) -> Result<String> {
    let output = sanitized_git_command(repo, arguments)?;
    if !output.status.success() {
        anyhow::bail!("Git authority observation failed");
    }
    let value = std::str::from_utf8(&output.stdout)?.trim();
    if value.is_empty() || value.lines().count() != 1 {
        anyhow::bail!("Git authority observation returned no unique value");
    }
    Ok(value.to_owned())
}

fn git_tracked_source_digest(repo: &Path) -> Result<(String, String, u64)> {
    let output = sanitized_git_command(repo, &["ls-files", "-s", "-z", "--", "."])?;
    if !output.status.success() {
        anyhow::bail!("Git index snapshot failed");
    }
    let ignored_components = [
        ".git",
        ".gradle",
        ".kotlin",
        ".semantic-thread",
        "build",
        "target",
        "node_modules",
        ".idea",
        ".vscode",
    ]
    .into_iter()
    .map(ToOwned::to_owned)
    .collect::<BTreeSet<_>>();
    let exclusions = RepositoryExclusions::from_components(ignored_components);
    let mut rows = Vec::new();
    let mut seen_paths = BTreeSet::new();
    let mut source_bytes = 0_u64;
    for raw in output
        .stdout
        .split(|byte| *byte == 0)
        .filter(|row| !row.is_empty())
    {
        let tab = raw
            .iter()
            .position(|byte| *byte == b'\t')
            .context("Git index row has no path separator")?;
        let header = std::str::from_utf8(&raw[..tab])?;
        let fields = header.split(' ').collect::<Vec<_>>();
        if fields.len() != 3 || fields[2] != "0" {
            anyhow::bail!("Git index contains a non-stage-zero member");
        }
        let mode = fields[0];
        let git_object = fields[1];
        if !matches!(mode, "100644" | "100755" | "120000") {
            anyhow::bail!("Git index contains an unsupported member mode");
        }
        validate_git_object(git_object)?;
        let raw_path = std::str::from_utf8(&raw[tab + 1..])?;
        let path = Path::new(raw_path);
        let normalized = normalized_repository_path(path)?;
        if normalized != raw_path || !seen_paths.insert(normalized.clone()) {
            anyhow::bail!("Git index member path is noncanonical or duplicated");
        }
        let member = repo.join(path);
        let metadata = std::fs::symlink_metadata(&member)?;
        if mode == "120000" {
            if !metadata.file_type().is_symlink() {
                anyhow::bail!("Git link index/worktree kind mismatch");
            }
            let target = std::fs::read_link(&member)?;
            let target_bytes = symlink_target_bytes(&target)?;
            let target_text = target
                .to_str()
                .context("Git tracked link target is not valid UTF-8")?;
            let destination = contained_symlink_destination(path, &target)?;
            if semantic_sensitive_symlink_path(path, &exclusions)
                || semantic_sensitive_symlink_path(&destination, &exclusions)
            {
                anyhow::bail!("Git tracked link participates in semantic/build inputs");
            }
            let after = std::fs::symlink_metadata(&member)?;
            if !same_file_observation(&metadata, &after) {
                anyhow::bail!("Git tracked link changed while hashing");
            }
            source_bytes = source_bytes.saturating_add(target_bytes.len() as u64);
            rows.push(json!({
                "path":normalized,
                "mode":mode,
                "gitObject":git_object,
                "kind":"SYMLINK",
                "size":target_bytes.len(),
                "sha256":hash_bytes(&target_bytes),
                "target":target_text,
            }));
        } else {
            if !metadata.is_file() || metadata.file_type().is_symlink() {
                anyhow::bail!("Git file index/worktree kind mismatch");
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let executable = metadata.permissions().mode() & 0o111 != 0;
                if executable != (mode == "100755") {
                    anyhow::bail!("Git file executable mode mismatch");
                }
            }
            let bytes = std::fs::read(&member)?;
            let after = std::fs::symlink_metadata(&member)?;
            if !same_file_observation(&metadata, &after) || after.len() != bytes.len() as u64 {
                anyhow::bail!("Git tracked file changed while hashing");
            }
            source_bytes = source_bytes.saturating_add(bytes.len() as u64);
            rows.push(json!({
                "path":normalized,
                "mode":mode,
                "gitObject":git_object,
                "kind":"FILE",
                "size":bytes.len(),
                "sha256":hash_bytes(&bytes),
            }));
        }
    }
    let index_digest = canonical_hash_with_newline(&rows)?;
    let index = json!({
        "schema":"codeclew.git-index-snapshot/0.1",
        "members":rows,
        "digest":index_digest,
    });
    let source_tree_sha256 = canonical_hash_with_newline(&json!({
        "schema":"codeclew.git-tracked-source/0.1",
        "index":index,
    }))?;
    Ok((source_tree_sha256, index_digest, source_bytes))
}

fn canonical_hash_with_newline<T: serde::Serialize>(value: &T) -> Result<String> {
    let mut bytes = canonical_bytes(value)?;
    bytes.push(b'\n');
    Ok(hash_bytes(&bytes))
}

fn validate_git_object(value: &str) -> Result<()> {
    if !matches!(value.len(), 40 | 64)
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        anyhow::bail!("malformed Git object identity");
    }
    Ok(())
}

fn same_file_observation(before: &std::fs::Metadata, after: &std::fs::Metadata) -> bool {
    if before.file_type() != after.file_type() || before.len() != after.len() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        before.dev() == after.dev()
            && before.ino() == after.ino()
            && before.mtime() == after.mtime()
            && before.mtime_nsec() == after.mtime_nsec()
    }
    #[cfg(not(unix))]
    true
}

fn validate_repository_tree_paths(
    repo: &Path,
    exclusions: &RepositoryExclusions,
) -> Result<Vec<Value>> {
    let tracked_symlinks = tracked_git_symlinks(repo)?;
    let mut link_objects = Vec::new();
    for entry in WalkDir::new(repo)
        .follow_links(false)
        .sort_by_file_name()
        .into_iter()
        .filter_entry(|entry| {
            entry
                .path()
                .strip_prefix(repo)
                .is_ok_and(|relative| !exclusions.excludes(relative))
        })
    {
        let entry = entry?;
        let relative = entry.path().strip_prefix(repo)?;
        let metadata = std::fs::symlink_metadata(entry.path())?;
        if metadata.file_type().is_symlink() {
            let normalized = normalized_repository_path(relative)?;
            if !tracked_symlinks.contains(&normalized) {
                anyhow::bail!("repository contains an untracked symbolic link");
            }
            let target = std::fs::read_link(entry.path())?;
            let target_bytes = symlink_target_bytes(&target)?;
            let destination = contained_symlink_destination(relative, &target)?;
            if semantic_sensitive_symlink_path(relative, exclusions)
                || semantic_sensitive_symlink_path(&destination, exclusions)
            {
                anyhow::bail!(
                    "tracked symbolic link participates in source/build/generated/cache inputs"
                );
            }
            link_objects.push(json!({
                "path":normalized,
                "targetBytesDigest":hash_bytes(&target_bytes),
                "targetSizeBytes":target_bytes.len(),
                "containedDestination":normalized_repository_path(&destination)?,
            }));
            continue;
        }
        if relative.as_os_str().is_empty() || metadata.is_dir() || metadata.is_file() {
            continue;
        }
        anyhow::bail!("source/build input tree contains a special filesystem object");
    }
    link_objects.sort_by(|left, right| left["path"].as_str().cmp(&right["path"].as_str()));
    Ok(link_objects)
}

fn tracked_git_symlinks(repo: &Path) -> Result<BTreeSet<String>> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["ls-files", "--stage", "-z", "--", "."])
        .output()?;
    if !output.status.success() {
        anyhow::bail!("cannot establish tracked symbolic-link authority");
    }
    let mut paths = BTreeSet::new();
    for row in output
        .stdout
        .split(|byte| *byte == 0)
        .filter(|row| !row.is_empty())
    {
        let tab = row
            .iter()
            .position(|byte| *byte == b'\t')
            .context("git index row has no path separator")?;
        let header = std::str::from_utf8(&row[..tab])?;
        if header.split_whitespace().next() != Some("120000") {
            continue;
        }
        let path = std::str::from_utf8(&row[tab + 1..])?;
        paths.insert(normalized_repository_path(Path::new(path))?);
    }
    Ok(paths)
}

fn normalized_repository_path(path: &Path) -> Result<String> {
    let mut parts = Vec::new();
    for component in path.components() {
        match component {
            Component::Normal(value) => parts.push(
                value
                    .to_str()
                    .context("repository path is not valid UTF-8")?
                    .to_owned(),
            ),
            Component::CurDir => {}
            _ => anyhow::bail!("repository path is not normalized and contained"),
        }
    }
    if parts.is_empty() {
        anyhow::bail!("repository path has no member identity");
    }
    Ok(parts.join("/"))
}

fn symlink_target_bytes(target: &Path) -> Result<Vec<u8>> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        let bytes = target.as_os_str().as_bytes();
        if bytes.is_empty() {
            anyhow::bail!("symbolic-link target is empty");
        }
        Ok(bytes.to_vec())
    }
    #[cfg(not(unix))]
    {
        let target = target
            .to_str()
            .context("symbolic-link target is not valid UTF-8")?;
        if target.is_empty() {
            anyhow::bail!("symbolic-link target is empty");
        }
        Ok(target.as_bytes().to_vec())
    }
}

fn contained_symlink_destination(link: &Path, target: &Path) -> Result<PathBuf> {
    if target.is_absolute() {
        anyhow::bail!("symbolic-link target is absolute");
    }
    let mut parts = link
        .parent()
        .into_iter()
        .flat_map(Path::components)
        .filter_map(|component| match component {
            Component::Normal(value) => Some(value.to_os_string()),
            Component::CurDir => None,
            _ => None,
        })
        .collect::<Vec<_>>();
    for component in target.components() {
        match component {
            Component::Normal(value) => parts.push(value.to_os_string()),
            Component::CurDir => {}
            Component::ParentDir => {
                if parts.pop().is_none() {
                    anyhow::bail!("symbolic-link target escapes the repository");
                }
            }
            Component::RootDir | Component::Prefix(_) => {
                anyhow::bail!("symbolic-link target escapes the repository")
            }
        }
    }
    if parts.is_empty() {
        anyhow::bail!("symbolic-link target resolves to the repository root");
    }
    Ok(parts.into_iter().collect())
}

fn repository_exclusions(
    repo: &Path,
    ignored_components: &BTreeSet<String>,
) -> RepositoryExclusions {
    repo_owned_git_exclusions(repo, ignored_components.clone())
        .unwrap_or_else(|_| RepositoryExclusions::from_components(ignored_components.clone()))
}

fn semantic_sensitive_symlink_path(path: &Path, exclusions: &RepositoryExclusions) -> bool {
    if exclusions.excludes(path) {
        return true;
    }
    let sensitive_components = [
        ".mvn",
        "build-logic",
        "buildSrc",
        "generated",
        "generated-sources",
        "generated-test-sources",
        "gradle",
    ];
    if path.components().any(|component| match component {
        Component::Normal(value) => value
            .to_str()
            .is_some_and(|value| sensitive_components.contains(&value)),
        _ => false,
    }) {
        return true;
    }
    let file_name = path.file_name().and_then(|value| value.to_str());
    if matches!(
        file_name,
        Some(
            "build.gradle"
                | "build.gradle.kts"
                | "settings.gradle"
                | "settings.gradle.kts"
                | "gradle.properties"
                | "libs.versions.toml"
                | "pom.xml"
                | "gradlew"
                | "gradlew.bat"
                | "mvnw"
                | "mvnw.cmd"
        )
    ) {
        return true;
    }
    matches!(
        path.extension().and_then(|value| value.to_str()),
        Some("java" | "kt" | "kts")
    )
}

fn tracked_symlink_manifest_digest(tracked_symlinks: &[Value]) -> Result<String> {
    canonical_hash(&json!({
        "schema":"codeclew.tracked-symlink-manifest/0.1",
        "members":tracked_symlinks,
    }))
}

fn k1_repository_tree_digest(
    sources: &[SourceInput],
    tracked_symlinks: &[Value],
) -> Result<String> {
    canonical_hash(&json!({
        "schema":"codeclew.repository-tree/0.1",
        "members":sources,
        "trackedSymlinkObjects":tracked_symlinks,
    }))
}

#[allow(clippy::too_many_arguments)]
fn verify_repository_unchanged(
    repo: &Path,
    ignored_components: &BTreeSet<String>,
    project: &Value,
    expected_sources: &[SourceInput],
    expected_tree_digest: &str,
    expected_git_status_digest: Option<&str>,
    expected_generated_boundaries: &[Value],
    expected_tracked_symlinks: &[Value],
) -> Result<()> {
    let exclusions = repository_exclusions(repo, ignored_components);
    let current_tracked_symlinks = validate_repository_tree_paths(repo, &exclusions)?;
    if canonical_bytes(&current_tracked_symlinks)?
        != canonical_bytes(&expected_tracked_symlinks.to_vec())?
    {
        anyhow::bail!("tracked symlink object state changed during analysis");
    }
    let (mut current_sources, _, _) = snapshot_repository(repo, &exclusions)?;
    let boundaries = augment_generated_sources(repo, project, &mut current_sources)?;
    if canonical_bytes(&boundaries)? != canonical_bytes(&expected_generated_boundaries)? {
        anyhow::bail!("generated-source boundary state changed during analysis");
    }
    current_sources.sort_by(|left, right| left.artifact_id.cmp(&right.artifact_id));
    let current_tree_digest =
        k1_repository_tree_digest(&current_sources, &current_tracked_symlinks)?;
    if current_tree_digest != expected_tree_digest
        || canonical_bytes(&current_sources)? != canonical_bytes(&expected_sources)?
    {
        anyhow::bail!("source/build/generated snapshot changed during analysis");
    }
    if let Some(expected) = expected_git_status_digest
        && git_status_digest(repo)?.as_deref() != Some(expected)
    {
        anyhow::bail!("repository Git status changed during analysis");
    }
    Ok(())
}

fn augment_generated_sources(
    repo: &Path,
    project: &Value,
    sources: &mut Vec<SourceInput>,
) -> Result<Vec<Value>> {
    let mut roots = BTreeSet::new();
    for pointer in [
        "/generatedSourceRoots",
        "/generatedSourceConfiguration/roots",
    ] {
        for value in project
            .pointer(pointer)
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            roots.insert(
                value
                    .as_str()
                    .with_context(|| format!("{pointer} member must be a string"))?
                    .to_owned(),
            );
        }
    }
    let mut boundaries = Vec::new();
    let mut generated_files = 0usize;
    for raw_root in &roots {
        let relative_root = safe_relative(raw_root)?;
        let lexical_root = repo.join(&relative_root);
        let metadata = match std::fs::symlink_metadata(&lexical_root) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                boundaries.push(generated_boundary(
                    "GENERATED_SOURCE_ROOT_MISSING",
                    json!({"root":raw_root}),
                )?);
                continue;
            }
            Err(error) => return Err(error.into()),
        };
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            anyhow::bail!("generated source root must be a regular non-symlink directory");
        }
        let canonical_root = lexical_root.canonicalize()?;
        if !canonical_root.starts_with(repo) {
            anyhow::bail!("generated source root escapes the source checkout");
        }
        for entry in WalkDir::new(&canonical_root)
            .follow_links(false)
            .sort_by_file_name()
        {
            let entry = entry?;
            let metadata = std::fs::symlink_metadata(entry.path())?;
            if metadata.file_type().is_symlink() {
                anyhow::bail!("generated source tree contains a symlink");
            }
            if !metadata.is_file() {
                continue;
            }
            let extension = entry
                .path()
                .extension()
                .and_then(|value| value.to_str())
                .unwrap_or_default();
            if !matches!(extension, "kt" | "kts" | "java") {
                continue;
            }
            let canonical = entry.path().canonicalize()?;
            if !canonical.starts_with(repo) {
                anyhow::bail!("generated source file escapes the source checkout");
            }
            let relative = repo.relative_path(&canonical)?;
            let bytes = std::fs::read(&canonical)?;
            let artifact_id = format!("source:{relative}");
            let input = SourceInput {
                artifact_id: artifact_id.clone(),
                normalized_path: relative,
                content_digest: hash_bytes(&bytes),
                size_bytes: bytes.len() as u64,
                origin: "GENERATED".to_owned(),
            };
            if let Some(existing) = sources
                .iter_mut()
                .find(|source| source.artifact_id == artifact_id)
            {
                *existing = input;
            } else {
                sources.push(input);
            }
            generated_files += 1;
        }
    }
    if !roots.is_empty() && generated_files == 0 {
        boundaries.push(generated_boundary(
            "GENERATED_SOURCE_SET_EMPTY_OR_UNMATERIALIZED",
            json!({"rootCount":roots.len()}),
        )?);
    }
    Ok(boundaries)
}

trait RepositoryRelativePath {
    fn relative_path(&self, child: &Path) -> Result<String>;
}

impl RepositoryRelativePath for Path {
    fn relative_path(&self, child: &Path) -> Result<String> {
        let relative = child.strip_prefix(self)?;
        let normalized = relative
            .components()
            .map(|component| component.as_os_str().to_string_lossy())
            .collect::<Vec<_>>()
            .join("/");
        if normalized.is_empty() {
            anyhow::bail!("generated source has no repository-relative path");
        }
        Ok(normalized)
    }
}

fn safe_relative(value: &str) -> Result<PathBuf> {
    let path = Path::new(value);
    if value.is_empty()
        || value.split('/').any(|part| part.is_empty())
        || path.components().any(|component| {
            matches!(
                component,
                Component::Prefix(_)
                    | Component::RootDir
                    | Component::ParentDir
                    | Component::CurDir
            )
        })
    {
        anyhow::bail!("generated source path is not normalized and contained");
    }
    Ok(path.to_path_buf())
}

fn generated_boundary(code: &str, details: Value) -> Result<Value> {
    Ok(json!({
        "boundaryId":canonical_hash(&json!({"domain":"generated-sources","code":code,"details":details}))?,
        "kindUri":format!("codeclew.boundary/kotlin-generated-sources/{}/1",uri_component(code)),
        "consequence":"ENUMERATION_INCOMPLETE",
        "origin":Value::Null,
        "provider":"KOTLIN_PROJECT_MODEL",
        "applicability":global_boundary_applicability(BOUNDARY_EFFECT_TOPOLOGY),
        "details":{"code":code,"providerDetails":details},
    }))
}

fn uri_component(value: &str) -> String {
    let mut result = String::new();
    for character in value.chars() {
        if character.is_ascii_alphanumeric() {
            result.push(character.to_ascii_lowercase());
        } else if !result.ends_with('-') {
            result.push('-');
        }
    }
    result.trim_matches('-').to_owned()
}

struct RangeSource {
    input: SourceInput,
    bytes: Vec<u8>,
}

struct SourceRangeValidator {
    by_path: BTreeMap<String, RangeSource>,
    path_by_artifact: BTreeMap<String, String>,
}

impl SourceRangeValidator {
    fn new(repo: &Path, sources: &[SourceInput]) -> Result<Self> {
        let repo = repo.canonicalize()?;
        let mut by_path = BTreeMap::new();
        let mut path_by_artifact = BTreeMap::new();
        for source in sources {
            let extension = Path::new(&source.normalized_path)
                .extension()
                .and_then(|value| value.to_str())
                .unwrap_or_default();
            if !matches!(extension, "kt" | "kts" | "java") {
                continue;
            }
            let relative = safe_relative(&source.normalized_path)?;
            let lexical = repo.join(relative);
            let metadata = std::fs::symlink_metadata(&lexical)?;
            if metadata.file_type().is_symlink() || !metadata.is_file() {
                anyhow::bail!("source range input must be a regular non-symlink file");
            }
            let canonical = lexical.canonicalize()?;
            if !canonical.starts_with(&repo) {
                anyhow::bail!("source range input escapes the source checkout");
            }
            let bytes = std::fs::read(&canonical)?;
            if bytes.len() as u64 != source.size_bytes
                || hash_bytes(&bytes) != source.content_digest
                || std::str::from_utf8(&bytes).is_err()
            {
                anyhow::bail!("source range input differs from the exact UTF-8 snapshot");
            }
            if path_by_artifact
                .insert(source.artifact_id.clone(), source.normalized_path.clone())
                .is_some()
                || by_path
                    .insert(
                        source.normalized_path.clone(),
                        RangeSource {
                            input: source.clone(),
                            bytes,
                        },
                    )
                    .is_some()
            {
                anyhow::bail!("source range input identity is duplicated");
            }
        }
        Ok(Self {
            by_path,
            path_by_artifact,
        })
    }

    fn range_from_provider(&self, value: &Value, required: bool) -> Result<Value> {
        let Some(path) = value.get("file").and_then(Value::as_str) else {
            if required {
                anyhow::bail!("proof-bearing compiler row has no source file");
            }
            return Ok(Value::Null);
        };
        let Some(source) = self.by_path.get(path) else {
            if required {
                anyhow::bail!("proof-bearing compiler row is outside the exact source snapshot");
            }
            return Ok(Value::Null);
        };
        let (Some(start), Some(end)) = (
            value.get("start").and_then(Value::as_u64),
            value.get("end").and_then(Value::as_u64),
        ) else {
            if required {
                anyhow::bail!("proof-bearing compiler row has no exact byte range");
            }
            return Ok(Value::Null);
        };
        self.validate_offsets(source, start, end)?;
        Ok(json!({
            "artifactId":source.input.artifact_id,
            "artifactContentDigest":source.input.content_digest,
            "startByte":start,
            "endByte":end,
        }))
    }

    fn validate_envelope_range(&self, range: &Value) -> Result<()> {
        let artifact = range
            .get("artifactId")
            .and_then(Value::as_str)
            .context("source range has no artifactId")?;
        let path = self
            .path_by_artifact
            .get(artifact)
            .context("source range artifact is outside the exact snapshot")?;
        let source = self
            .by_path
            .get(path)
            .context("source range artifact has no live UTF-8 content")?;
        if range.get("artifactContentDigest").and_then(Value::as_str)
            != Some(source.input.content_digest.as_str())
        {
            anyhow::bail!("source range content digest differs from the exact snapshot");
        }
        let start = range
            .get("startByte")
            .and_then(Value::as_u64)
            .context("source range has no startByte")?;
        let end = range
            .get("endByte")
            .and_then(Value::as_u64)
            .context("source range has no endByte")?;
        self.validate_offsets(source, start, end)
    }

    fn validate_offsets(&self, source: &RangeSource, start: u64, end: u64) -> Result<()> {
        if start > end || end > source.input.size_bytes {
            anyhow::bail!("source range is outside the exact source byte length");
        }
        let start = usize::try_from(start).context("source range start does not fit usize")?;
        let end = usize::try_from(end).context("source range end does not fit usize")?;
        let text =
            std::str::from_utf8(&source.bytes).context("source range input is not valid UTF-8")?;
        if !text.is_char_boundary(start) || !text.is_char_boundary(end) {
            anyhow::bail!("source range splits a UTF-8 code point");
        }
        Ok(())
    }
}

fn validate_adapter_ranges(output: &AdapterOutput, ranges: &SourceRangeValidator) -> Result<()> {
    for entity in &output.entities {
        if let Some(range) = entity
            .get("primaryDefinition")
            .filter(|value| !value.is_null())
        {
            ranges.validate_envelope_range(range)?;
        }
    }
    for occurrence in &output.occurrences {
        ranges.validate_envelope_range(
            occurrence
                .get("range")
                .context("proof-bearing occurrence has no range")?,
        )?;
    }
    for fact in &output.facts {
        ranges.validate_envelope_range(
            fact.get("range")
                .context("proof-bearing relation fact has no range")?,
        )?;
    }
    for boundary in &output.boundaries {
        if let Some(origin) = boundary.get("origin").filter(|value| !value.is_null()) {
            ranges.validate_envelope_range(origin)?;
        }
    }
    Ok(())
}

fn coarse_kind(kind: Option<&str>) -> &'static str {
    match kind.unwrap_or_default() {
        "CLASS" | "INTERFACE" | "OBJECT" | "TYPE_ALIAS" => "TYPE_LIKE",
        "FUNCTION" | "CONSTRUCTOR" => "CALLABLE",
        "PROPERTY" | "FIELD" => "FIELD_LIKE",
        "PARAMETER" | "LOCAL" => "VALUE_LIKE",
        _ => "VALUE_LIKE",
    }
}

#[cfg(test)]
mod adapter_local_tests {
    use super::*;
    use tempfile::tempdir;

    fn semantic_digest_fixture() -> AdapterOutput {
        AdapterOutput {
            schema: ADAPTER_OUTPUT_SCHEMA.to_owned(),
            adapter: AdapterIdentity {
                adapter_id: "codeclew.kotlin-k2".to_owned(),
                version: "0.1.0".to_owned(),
                binary_digest: hash_bytes(b"adapter"),
                language_id: "kotlin".to_owned(),
            },
            snapshot_input: SnapshotInput {
                repository_tree_digest: hash_bytes(b"tree"),
                vcs_revision: None,
                dirty: false,
                sources: Vec::new(),
                build_system_uri: "codeclew.build-system/gradle/1".to_owned(),
                build_model_digest: hash_bytes(b"model"),
                build_configuration_digest: hash_bytes(b"configuration"),
                dependency_graph_digest: hash_bytes(b"dependencies"),
                toolchain: json!({"toolUri":"codeclew.toolchain/kotlin-k2/1"}),
                targets: Vec::new(),
                relevant_environment: Vec::new(),
                generated_sources_manifest_digest: hash_bytes(b"generated"),
            },
            capability_descriptors: Vec::new(),
            entities: Vec::new(),
            occurrences: Vec::new(),
            facts: Vec::new(),
            boundaries: Vec::new(),
            compiler_receipt: json!({"status":"ACCEPTED"}),
            impact: json!({"status":"COMPLETE"}),
            cost: CostRecord {
                total_wall_micros: 0,
                repository_snapshot_micros: 0,
                build_discovery_micros: 0,
                cold_index_micros: 0,
                warm_index_micros: 0,
                adapter_micros: 0,
                query_micros: 0,
                source_bytes_read: 0,
                emitted_bytes: 0,
                stored_fact_bytes: 0,
                model_visible_source_bytes: 0,
                cache_requests: 0,
                cache_hits: 0,
                provider_processing_micros: 0,
            },
            output_digest: String::new(),
        }
    }

    #[test]
    fn operational_profiles_are_attempt_only_and_semantic_digest_is_unchanged() {
        let output = semantic_digest_fixture();
        let output_before = serde_json::to_value(&output).unwrap();
        let digest_before = kotlin_k1::semantic_output_digest(&output).unwrap();
        let mut context = RunContext::new();
        context.cache["existing"] = Value::String("preserved".to_owned());
        let profile = RequestProfile {
            compiler_index: Some(clew::worker::CompilerIndexProfile {
                backend: clew::worker::CompilerIndexBackend::BtaPersistent,
                status: clew::worker::CompilerIndexStatus::Incremental,
                valid: true,
                total_micros: 120,
                compiler_micros: 80,
                fir_extraction_micros: 30,
                total_files: 5,
                compiled_files: 2,
                reused_files: 3,
                recovered: false,
                fallback_used: false,
                graph_digest: Some("a".repeat(64)),
            }),
            project_model_cache: Some(clew::worker::ProjectModelCacheProfile {
                status: clew::worker::ProjectModelCacheStatus::PersistentHit,
                publish_outcome: clew::worker::ProjectModelPublishOutcome::NotAttempted,
                publish_invalid_reason: clew::worker::ProjectModelInvalidReason::NotApplicable,
                total_micros: 110,
                key_micros: 20,
                load_micros: 80,
                extraction_micros: 0,
                publish_micros: 0,
                persistent_configured: true,
                published: false,
            }),
            ..RequestProfile::default()
        };

        retain_operational_profiles(&mut context, &profile);

        assert_eq!(serde_json::to_value(&output).unwrap(), output_before);
        assert_eq!(
            kotlin_k1::semantic_output_digest(&output).unwrap(),
            digest_before
        );
        assert_eq!(context.cache["existing"], "preserved");
        assert_eq!(context.cache["compilerIndex"]["backend"], "BTA_PERSISTENT");
        assert_eq!(context.cache["compilerIndex"]["status"], "INCREMENTAL");
        assert_eq!(context.cache["compilerIndex"]["valid"], true);
        assert_eq!(
            context.cache["projectModelCache"]["status"],
            "PERSISTENT_HIT"
        );
        assert_eq!(context.cache["projectModelCache"]["totalMicros"], 110);
        assert_eq!(
            context.cache["projectModelCache"]["persistentConfigured"],
            true
        );
        assert_eq!(
            context.cache["projectModelCache"]["publishInvalidReason"],
            "NOT_APPLICABLE"
        );

        let cache_before = context.cache.clone();
        retain_operational_profiles(&mut context, &RequestProfile::default());
        assert_eq!(context.cache, cache_before);
    }

    fn git(repo: &Path, arguments: &[&str]) {
        let status = Command::new("git")
            .arg("-C")
            .arg(repo)
            .args(arguments)
            .status()
            .unwrap();
        assert!(status.success());
    }

    fn initialized_git_repository() -> tempfile::TempDir {
        let repo = tempdir().unwrap();
        git(repo.path(), &["init", "-q"]);
        repo
    }

    fn committed_git_repository() -> tempfile::TempDir {
        let repo = initialized_git_repository();
        std::fs::create_dir_all(repo.path().join("src")).unwrap();
        std::fs::write(repo.path().join("src/Main.kt"), b"fun main() = Unit\n").unwrap();
        std::fs::write(
            repo.path().join("settings.gradle.kts"),
            b"rootProject.name = \"fixture\"\n",
        )
        .unwrap();
        git(repo.path(), &["add", "src/Main.kt", "settings.gradle.kts"]);
        git(
            repo.path(),
            &[
                "-c",
                "user.name=Codeclew Test",
                "-c",
                "user.email=codeclew@example.invalid",
                "commit",
                "-q",
                "-m",
                "fixture",
            ],
        );
        repo
    }

    fn prepared_refusal_args(repository: &Path, authority_root: &Path) -> (Args, PreparedRefusal) {
        let observation = git_source_observation(repository).unwrap();
        let mut refusal = PreparedRefusal {
            schema: kotlin_k1::PREPARED_REFUSAL_SCHEMA.to_owned(),
            series_id: kotlin_k1::K1_SERIES_ID.to_owned(),
            cohort: "QUALIFICATION".to_owned(),
            entry: "K1-Q01".to_owned(),
            commit: observation.head,
            git_tree: observation.tree,
            selected_compilation: ":/main".to_owned(),
            build_dsl: "GRADLE_KOTLIN_DSL".to_owned(),
            failure_stage: "DEPENDENCY_PREPARATION".to_owned(),
            reason_code: "DEPENDENCY_CLOSURE_UNAVAILABLE".to_owned(),
            safe_detail_digest: hash_bytes(b"safe preparation diagnostic"),
            cost: kotlin_k1::PreparedRefusalCost {
                wall_micros: 123,
                stdout_bytes: 5,
                stderr_bytes: 7,
                exit_code: 1,
            },
            sandbox_profile_sha256: hash_bytes(b"sandbox"),
            source_tree_sha256: observation.source_tree_sha256,
            candidate_tools_sha256: hash_bytes(b"candidate-tools"),
            build_input_digest: hash_bytes(b"build-input"),
            preparation_receipt_digest: hash_bytes(b"prepare-receipt"),
            object_digest: String::new(),
        };
        refusal.object_digest = canonical_hash(&refusal).unwrap();
        let path = authority_root.join("PREPARED_REFUSAL.json");
        let mut raw = canonical_bytes(&refusal).unwrap();
        raw.push(b'\n');
        std::fs::write(&path, &raw).unwrap();
        let args = Args {
            repo: repository.canonicalize().unwrap(),
            compilation: ":/main".to_owned(),
            seed_entity: None,
            max_depth: 2,
            max_entities: 128,
            agent_output: false,
            state_root: None,
            build_state_root: None,
            attempt_output: None,
            run_phase: RunPhase::Cold,
            prepared_refusal: Some(path.canonicalize().unwrap()),
            prepared_refusal_sha256: Some(hash_bytes(&raw)),
            entry_id: Some(refusal.entry.clone()),
            candidate_tools_sha256: Some(refusal.candidate_tools_sha256.clone()),
            build_input_digest: Some(refusal.build_input_digest.clone()),
            preparation_receipt_digest: Some(refusal.preparation_receipt_digest.clone()),
        };
        (args, refusal)
    }

    #[test]
    fn git_clean_observation_never_executes_repository_filters_and_detects_dirty_states() {
        let repository = initialized_git_repository();
        let marker_root = tempdir().unwrap();
        let marker = marker_root.path().join("filter-marker");
        std::fs::create_dir_all(repository.path().join("src")).unwrap();
        std::fs::write(
            repository.path().join(".gitattributes"),
            b"src/Main.kt filter=marker\n",
        )
        .unwrap();
        std::fs::write(
            repository.path().join("src/Main.kt"),
            b"fun main() = Unit\n",
        )
        .unwrap();
        std::fs::write(repository.path().join(".gitignore"), b"runtime/\n").unwrap();
        git(repository.path(), &["config", "filter.marker.clean", "cat"]);
        git(repository.path(), &["add", "."]);
        git(
            repository.path(),
            &[
                "-c",
                "user.name=Codeclew Test",
                "-c",
                "user.email=codeclew@example.invalid",
                "commit",
                "-q",
                "-m",
                "filter fixture",
            ],
        );
        git(
            repository.path(),
            &[
                "config",
                "filter.marker.clean",
                &format!("sh -c 'printf invoked > {}; cat'", marker.display()),
            ],
        );

        let clean = git_source_observation(repository.path()).unwrap();
        assert!(clean.clean);
        assert_eq!(clean.status_digest, hash_bytes(b""));
        assert!(!marker.exists());

        std::fs::create_dir(repository.path().join("runtime")).unwrap();
        std::fs::write(repository.path().join("runtime/cache.bin"), b"first").unwrap();
        let ignored_runtime = git_source_observation(repository.path()).unwrap();
        assert!(ignored_runtime.clean);
        assert_eq!(ignored_runtime.status_digest, clean.status_digest);
        std::fs::write(repository.path().join("runtime/cache.bin"), b"second").unwrap();
        assert_eq!(
            git_source_observation(repository.path())
                .unwrap()
                .status_digest,
            clean.status_digest
        );

        std::fs::write(
            repository.path().join("src/Main.kt"),
            b"fun dirty() = Unit\n",
        )
        .unwrap();
        let modified = git_source_observation(repository.path()).unwrap();
        assert!(!modified.clean);
        assert_ne!(modified.status_digest, clean.status_digest);
        assert!(!marker.exists());

        std::fs::write(
            repository.path().join("src/Main.kt"),
            b"fun main() = Unit\n",
        )
        .unwrap();
        std::fs::write(repository.path().join("untracked.txt"), b"untracked\n").unwrap();
        assert!(!git_source_observation(repository.path()).unwrap().clean);
        assert!(!marker.exists());
        std::fs::remove_file(repository.path().join("untracked.txt")).unwrap();
        std::fs::remove_file(repository.path().join("src/Main.kt")).unwrap();
        assert!(!git_source_observation(repository.path()).unwrap().clean);
        assert!(!marker.exists());
    }

    #[test]
    fn git_clean_observation_accepts_explicit_external_source_set() {
        let Some(root) = std::env::var_os("CODECLEW_K1_TEST_SOURCE_SET").map(PathBuf::from) else {
            return;
        };
        for number in 1..=6 {
            let entry = format!("K1-Q{number:02}");
            let observation = git_source_observation(&root.join(&entry)).unwrap();
            assert!(observation.clean, "{entry} was not observed clean");
            assert_eq!(observation.status_digest, hash_bytes(b""), "{entry}");
        }
    }

    fn terminal_from_context(context: &RunContext) -> KotlinAttempt {
        let boundary = json!({
            "boundaryId":hash_bytes(format!("{}:{}",context.stage,context.reason_code).as_bytes()),
            "kindUri":format!("codeclew.boundary/kotlin-k1/{}/1",context.reason_code.to_ascii_lowercase().replace('_',"-")),
            "consequence":"PROOF_INVALID",
        });
        KotlinAttempt::terminal_with_detail_digest(
            context.status,
            context.stage,
            context.reason_code,
            context.detail_digest_override.as_deref().unwrap(),
            context.selected_inputs.clone(),
            context.snapshot.clone(),
            context.provenance.clone(),
            vec![boundary],
            context.cache.clone(),
            context.telemetry.clone(),
        )
        .unwrap()
    }

    fn assert_closure_obligation(impact: &Value) {
        let boundaries = impact["boundaries"].as_array().unwrap();
        let obligations = impact["mandatoryObligations"].as_array().unwrap();
        let closure = obligations
            .iter()
            .filter(|obligation| {
                obligation.get("kind").and_then(Value::as_str)
                    == Some("codeclew.obligation/impact-closure-completeness/1")
            })
            .collect::<Vec<_>>();
        assert_eq!(closure.len(), 1);
        assert_eq!(closure[0]["mandatory"], true);
        assert_eq!(
            closure[0]["status"],
            if boundaries.is_empty() {
                "SATISFIED"
            } else {
                "UNKNOWN"
            }
        );
        assert_eq!(
            closure[0]["providerPayload"]["queryRelevantBoundarySetDigest"],
            canonical_hash(&boundaries).unwrap()
        );
    }

    fn prepared_build_state() -> (tempfile::TempDir, tempfile::TempDir) {
        let repository = tempdir().unwrap();
        let state = tempdir().unwrap();
        let root = state.path().canonicalize().unwrap();
        std::fs::create_dir(root.join("gradle-user-home")).unwrap();
        std::fs::create_dir(root.join("maven-repository")).unwrap();
        let mut manifest = json!({
            "schema":BUILD_STATE_MANIFEST_SCHEMA,
            "seriesId":"K1_ADAPTER_FIXTURE",
            "cohort":"FIXTURE",
            "toolchain":{"fixture":hash_bytes(b"toolchain")},
            "repositories":[],
            "gradleUserHomeTreeDigest":hash_bytes(b""),
            "mavenLocalRepositoryTreeDigest":hash_bytes(b""),
            "files":[],
            "seedDigest":"",
        });
        let mut seed_body = canonical_bytes(&manifest).unwrap();
        seed_body.push(b'\n');
        manifest["seedDigest"] = Value::String(hash_bytes(&seed_body));
        let mut manifest_bytes = canonical_bytes(&manifest).unwrap();
        manifest_bytes.push(b'\n');
        std::fs::write(root.join(BUILD_STATE_MANIFEST_FILE), &manifest_bytes).unwrap();
        std::fs::write(
            root.join(BUILD_STATE_SEED_FILE),
            format!("{}\n", hash_bytes(&manifest_bytes)),
        )
        .unwrap();
        (repository, state)
    }

    fn validator(source: &[u8]) -> (tempfile::TempDir, SourceRangeValidator, SourceInput) {
        let repo = tempdir().unwrap();
        std::fs::write(repo.path().join("Source.kt"), source).unwrap();
        let input = SourceInput {
            artifact_id: "source:Source.kt".to_owned(),
            normalized_path: "Source.kt".to_owned(),
            content_digest: hash_bytes(source),
            size_bytes: source.len() as u64,
            origin: "USER".to_owned(),
        };
        let validator =
            SourceRangeValidator::new(repo.path(), std::slice::from_ref(&input)).unwrap();
        (repo, validator, input)
    }

    #[test]
    fn range_validator_accepts_exact_utf8_byte_boundaries() {
        let (_repo, validator, _) = validator("fun café() = 1\n".as_bytes());
        let range = validator
            .range_from_provider(&json!({"file":"Source.kt","start":4,"end":9}), true)
            .unwrap();
        assert_eq!(range["startByte"], 4);
        assert_eq!(range["endByte"], 9);
        validator.validate_envelope_range(&range).unwrap();
    }

    #[test]
    fn range_validator_rejects_out_of_bounds_and_split_utf8() {
        let (_repo, validator, input) = validator("éx".as_bytes());
        assert!(
            validator
                .range_from_provider(&json!({"file":"Source.kt","start":0,"end":4}), true)
                .is_err()
        );
        assert!(
            validator
                .range_from_provider(&json!({"file":"Source.kt","start":1,"end":2}), true)
                .is_err()
        );
        assert!(
            validator
                .validate_envelope_range(&json!({
                    "artifactId":input.artifact_id,
                    "artifactContentDigest":input.content_digest,
                    "startByte":0,
                    "endByte":99,
                }))
                .is_err()
        );
    }

    #[test]
    fn requested_seed_resolves_exact_opaque_or_unique_compiler_identity() {
        let entities = vec![
            json!({"opaqueId":"z","resolution":"RESOLVED","primaryDefinition":{"artifactId":"source:z"},"languagePayload":{"compilerCallableId":"p/Z.run"}}),
            json!({"opaqueId":"a","resolution":"RESOLVED","primaryDefinition":{"artifactId":"source:a"},"languagePayload":{"compilerCallableId":"p/A.run"}}),
            json!({"opaqueId":"ignored","resolution":"UNRESOLVED","primaryDefinition":{"artifactId":"source:i"}}),
        ];
        let facts = vec![json!({"owner":"z","target":"a"})];
        assert_eq!(resolve_requested_seed("a", &entities, &facts).unwrap(), "a");
        assert_eq!(
            resolve_requested_seed("p/Z.run", &entities, &facts).unwrap(),
            "z"
        );
        assert!(resolve_requested_seed("missing", &entities, &facts).is_err());
    }

    #[test]
    fn requested_compiler_alias_never_uses_incident_facts_to_guess_an_overload() {
        let entities = vec![
            json!({"opaqueId":"callable:p/run#jvm:()V","resolution":"RESOLVED","primaryDefinition":{"artifactId":"source:a"},"languagePayload":{"declarationKind":"FUNCTION","compilerCallableId":"p/run"}}),
            json!({"opaqueId":"callable:p/run#jvm:(I)V","resolution":"RESOLVED","primaryDefinition":{"artifactId":"source:b"},"languagePayload":{"declarationKind":"FUNCTION","compilerCallableId":"p/run"}}),
        ];
        let facts = vec![json!({"owner":"callable:p/run#jvm:()V","target":"other"})];
        assert!(resolve_requested_seed("p/run", &entities, &facts).is_err());
        assert_eq!(
            resolve_requested_seed("callable:p/run#jvm:()V", &entities, &facts).unwrap(),
            "callable:p/run#jvm:()V"
        );
    }

    #[test]
    fn direct_cold_run_is_cache_optional_and_warm_run_requires_cache_and_seed() {
        assert!(validate_run_phase(RunPhase::Cold, None, true).is_ok());
        assert!(validate_run_phase(RunPhase::Cold, None, false).is_ok());
        assert!(validate_run_phase(RunPhase::Cold, Some("a"), false).is_ok());
        assert!(validate_run_phase(RunPhase::Warm, None, true).is_err());
        assert!(validate_run_phase(RunPhase::Warm, Some("a"), false).is_err());
        assert!(validate_run_phase(RunPhase::Warm, Some("a"), true).is_ok());
    }

    #[test]
    fn relaxed_agent_graph_lookup_never_intercepts_strict_warm() {
        let parse = |phase: &str| {
            Args::try_parse_from([
                "codeclew-kotlin-evidence",
                "--repo",
                "/tmp/repository",
                "--agent-output",
                "--seed-entity",
                "seed",
                "--run-phase",
                phase,
            ])
            .unwrap()
        };
        assert!(permits_relaxed_agent_graph_lookup(&parse("cold")));
        assert!(!permits_relaxed_agent_graph_lookup(&parse("warm")));
    }

    #[test]
    fn relation_endpoints_use_only_unique_compiler_descriptor_identities() {
        let graph = json!({"descriptors":[
            {"symbolIdentity":"callable:p/A.run#jvm:()V","compilerCallableId":"p/A.run"},
            {"symbolIdentity":"callable:p/B.run#jvm:(I)V","compilerCallableId":"p/B.run"},
            {"symbolIdentity":"callable:p/B.run#jvm:(J)V","compilerCallableId":"p/B.run"}
        ]});
        let identities = unique_descriptor_endpoint_identities(&graph);
        assert_eq!(
            identities.get("p/A.run").map(String::as_str),
            Some("callable:p/A.run#jvm:()V")
        );
        assert!(!identities.contains_key("p/B.run"));
        assert_eq!(
            identities
                .get("callable:p/B.run#jvm:(I)V")
                .map(String::as_str),
            Some("callable:p/B.run#jvm:(I)V")
        );
    }

    #[test]
    fn boundary_target_scope_preserves_exact_locality_and_fails_closed_on_missing_data() {
        let (exact, key) = boundary_target_scope(Some("callable:p/A.run#jvm:()V")).unwrap();
        assert_eq!(exact, BoundaryTargetScope::Exact);
        assert_eq!(
            key,
            Some(query_endpoint_key("callable:p/A.run#jvm:()V").unwrap())
        );

        for excluded in ["", "null", "<local>/value"] {
            let (scope, key) = boundary_target_scope(Some(excluded)).unwrap();
            assert_eq!(scope, BoundaryTargetScope::OutOfScope);
            assert!(key.is_none());
        }
        let (missing, key) = boundary_target_scope(None).unwrap();
        assert_eq!(missing, BoundaryTargetScope::Global);
        assert!(key.is_none());
    }

    #[test]
    fn exact_boundary_groups_preserve_owner_target_correlation() {
        let target_a = query_endpoint_key("p/A.target").unwrap();
        let target_b = query_endpoint_key("p/B.target").unwrap();
        let mut owners_by_target = BTreeMap::<BoundaryTargetBucket, BTreeSet<String>>::new();
        for (owner, target) in [
            ("p/A.owner", target_a.clone()),
            ("p/B.owner", target_b.clone()),
        ] {
            owners_by_target
                .entry(boundary_target_bucket(
                    BoundaryTargetScope::Exact,
                    Some(target),
                ))
                .or_default()
                .insert(query_endpoint_key(owner).unwrap());
        }

        assert_eq!(owners_by_target.len(), 2);
        assert_eq!(
            owners_by_target
                .get(&boundary_target_bucket(
                    BoundaryTargetScope::Exact,
                    Some(target_a),
                ))
                .unwrap(),
            &BTreeSet::from([query_endpoint_key("p/A.owner").unwrap()])
        );
        assert_eq!(
            boundary_target_bucket(BoundaryTargetScope::Exact, None).scope,
            BoundaryTargetScope::Global
        );
    }

    #[test]
    fn compact_boundary_frontier_excludes_only_terminal_depth_entities() {
        let entities = vec![
            json!({"opaqueId":"seed"}),
            json!({"opaqueId":"caller"}),
            json!({"opaqueId":"terminal"}),
            json!({"opaqueId":"malformed-depth"}),
        ];
        let affected = vec![
            json!({"entityId":"seed","depth":0}),
            json!({"entityId":"caller","depth":1}),
            json!({"entityId":"terminal","depth":2}),
            json!({"entityId":"malformed-depth","depth":"unknown"}),
        ];
        let frontier = traversable_boundary_frontier_entities(&entities, &affected, 2)
            .into_iter()
            .filter_map(|entity| entity["opaqueId"].as_str().map(str::to_owned))
            .collect::<BTreeSet<_>>();
        assert_eq!(
            frontier,
            BTreeSet::from([
                "seed".to_owned(),
                "caller".to_owned(),
                "malformed-depth".to_owned(),
            ])
        );
    }

    #[test]
    fn semantic_cache_and_build_state_roots_must_be_disjoint() {
        let root = tempdir().unwrap();
        let semantic = root.path().join("semantic");
        let build = root.path().join("build");
        std::fs::create_dir(&semantic).unwrap();
        std::fs::create_dir(&build).unwrap();
        assert!(ensure_external_roots_disjoint(&semantic, &build).is_ok());
        assert!(ensure_external_roots_disjoint(&semantic, &semantic).is_err());
        assert!(ensure_external_roots_disjoint(root.path(), &build).is_err());
    }

    #[test]
    fn build_state_manifest_and_marker_are_exact_self_sealed_inputs() {
        let (repository, state) = prepared_build_state();
        let repository = repository.path().canonicalize().unwrap();
        let root = state.path().canonicalize().unwrap();
        let identity = validate_build_state_root(&root, &repository).unwrap();
        assert_eq!(identity.root, root);
        assert!(identity.seed_digest.starts_with("sha256:"));
        assert!(identity.manifest_digest.starts_with("sha256:"));
        assert!(identity.marker_bytes_digest.starts_with("sha256:"));

        std::fs::write(root.join(BUILD_STATE_SEED_FILE), b"sha256:forged\n").unwrap();
        assert!(validate_build_state_root(&root, &repository).is_err());
    }

    #[test]
    fn worker_build_state_identity_must_match_adapter_prepared_manifest() {
        let prepared = PreparedBuildStateIdentity {
            root: PathBuf::from("/external/build-state"),
            seed_digest: hash_bytes(b"seed"),
            manifest_digest: hash_bytes(b"manifest"),
            marker_bytes_digest: hash_bytes(b"marker"),
        };
        let manifest = json!({
            "buildState":{
                "mode":"EXTERNAL",
                "seedDigest":prepared.seed_digest,
                "manifestDigest":prepared.manifest_digest,
                "markerBytesDigest":prepared.marker_bytes_digest,
                "namespaceDigest":hash_bytes(b"namespace"),
                "gradleUserHome":"gradle-user-home",
                "mavenLocalRepository":"maven-repository",
                "homeCredentials":"ISOLATED",
            }
        });
        validate_worker_build_state_identity(&manifest, &prepared).unwrap();
        let mut forged = manifest;
        forged["buildState"]["manifestDigest"] = Value::String(hash_bytes(b"forged"));
        assert!(validate_worker_build_state_identity(&forged, &prepared).is_err());
    }

    #[test]
    fn repository_local_build_state_is_explicitly_partial_and_unsealed() {
        let manifest = json!({
            "buildState":{
                "mode":"LEGACY_REPOSITORY_OWNED",
                "runtimeIsolation":"REPOSITORY_OWNED_LEGACY",
                "namespaceDigest":hash_bytes(b"namespace"),
                "gradleUserHome":"gradle-user-home",
                "mavenLocalRepository":"maven-repository",
                "homeCredentials":"INHERITED_LEGACY",
            }
        });
        validate_repository_local_build_state_identity(&manifest).unwrap();
        let mut forged = manifest;
        forged["buildState"]["seedDigest"] = Value::String(hash_bytes(b"forged-seal"));
        assert!(validate_repository_local_build_state_identity(&forged).is_err());
    }

    #[test]
    fn semantic_manifest_identity_ignores_disposable_classpath_mounts() {
        let first = json!({
            "schema":"kotlin-semantic-input-manifest/0.1",
            "orderedCompileClasspath":[
                "artifact:a.jar:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "artifact:b.jar:sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            ],
            "classpathAuthority":{
                "chosen":"MAVEN_DEPENDENCY_BUILD_CLASSPATH",
                "orderedDigest":hash_bytes(b"/private/runtime-one/a.jar\0/private/runtime-one/b.jar\0"),
            },
        });
        let mut second = first.clone();
        second["classpathAuthority"]["orderedDigest"] = Value::String(hash_bytes(
            b"/private/runtime-two/a.jar\0/private/runtime-two/b.jar\0",
        ));

        let stable_first = stable_semantic_input_manifest(&first).unwrap();
        let stable_second = stable_semantic_input_manifest(&second).unwrap();
        assert_eq!(stable_first, stable_second);
        assert_eq!(
            canonical_hash(&stable_first).unwrap(),
            canonical_hash(&stable_second).unwrap()
        );

        let mut reordered = second;
        reordered["orderedCompileClasspath"] = json!([
            "artifact:b.jar:sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            "artifact:a.jar:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        ]);
        assert_ne!(
            canonical_hash(&stable_first).unwrap(),
            canonical_hash(&stable_semantic_input_manifest(&reordered).unwrap()).unwrap()
        );
    }

    #[test]
    fn stable_project_model_digest_uses_normalized_semantic_authority() {
        let raw_manifest = json!({
            "orderedCompileClasspath":["artifact:a.jar:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"],
            "classpathAuthority":{"chosen":"MAVEN_DEPENDENCY_BUILD_CLASSPATH","orderedDigest":hash_bytes(b"raw-one")},
        });
        let mut second_manifest = raw_manifest.clone();
        second_manifest["classpathAuthority"]["orderedDigest"] =
            Value::String(hash_bytes(b"raw-two"));
        let stable_first = stable_semantic_input_manifest(&raw_manifest).unwrap();
        let stable_second = stable_semantic_input_manifest(&second_manifest).unwrap();
        let first_hash = canonical_hash(&stable_first).unwrap();
        let second_hash = canonical_hash(&stable_second).unwrap();
        let first_project = json!({
            "projectModelHash":hash_bytes(b"raw-project-one"),
            "classpathAuthority":raw_manifest["classpathAuthority"],
            "semanticInputManifest":raw_manifest,
            "semanticInputManifestHash":hash_bytes(b"raw-manifest-one"),
        });
        let second_project = json!({
            "projectModelHash":hash_bytes(b"raw-project-two"),
            "classpathAuthority":second_manifest["classpathAuthority"],
            "semanticInputManifest":second_manifest,
            "semanticInputManifestHash":hash_bytes(b"raw-manifest-two"),
        });
        assert_eq!(
            stable_project_model_digest(&first_project, &stable_first, &first_hash).unwrap(),
            stable_project_model_digest(&second_project, &stable_second, &second_hash).unwrap(),
        );
    }

    #[cfg(unix)]
    #[test]
    fn tracked_unrelated_symlink_is_hashed_without_dereference() {
        let repo = initialized_git_repository();
        std::fs::write(repo.path().join("AGENTS.md"), b"instructions\n").unwrap();
        std::os::unix::fs::symlink("AGENTS.md", repo.path().join("CLAUDE.md")).unwrap();
        git(repo.path(), &["add", "AGENTS.md", "CLAUDE.md"]);

        let exclusions = RepositoryExclusions::default();
        let first = validate_repository_tree_paths(repo.path(), &exclusions).unwrap();
        assert_eq!(first.len(), 1);
        assert_eq!(first[0]["path"], "CLAUDE.md");
        assert_eq!(first[0]["containedDestination"], "AGENTS.md");
        let first_digest = tracked_symlink_manifest_digest(&first).unwrap();
        std::fs::remove_file(repo.path().join("CLAUDE.md")).unwrap();
        std::os::unix::fs::symlink("README.md", repo.path().join("CLAUDE.md")).unwrap();
        let second = validate_repository_tree_paths(repo.path(), &exclusions).unwrap();
        assert_ne!(
            first_digest,
            tracked_symlink_manifest_digest(&second).unwrap()
        );
        assert_ne!(
            k1_repository_tree_digest(&[], &first).unwrap(),
            k1_repository_tree_digest(&[], &second).unwrap()
        );
    }

    #[cfg(unix)]
    #[test]
    fn source_and_escaping_symlinks_are_rejected() {
        let source_repo = initialized_git_repository();
        std::fs::write(source_repo.path().join("Source.kt"), b"fun source() = 1\n").unwrap();
        std::os::unix::fs::symlink("Source.kt", source_repo.path().join("Alias.kt")).unwrap();
        git(source_repo.path(), &["add", "Source.kt", "Alias.kt"]);
        assert!(
            validate_repository_tree_paths(source_repo.path(), &RepositoryExclusions::default())
                .is_err()
        );

        let escape_repo = initialized_git_repository();
        std::os::unix::fs::symlink("../outside.md", escape_repo.path().join("CLAUDE.md")).unwrap();
        git(escape_repo.path(), &["add", "CLAUDE.md"]);
        assert!(
            validate_repository_tree_paths(escape_repo.path(), &RepositoryExclusions::default())
                .is_err()
        );
    }

    #[cfg(unix)]
    #[test]
    fn ignored_runtime_subtrees_are_not_symlink_authority() {
        let repo = initialized_git_repository();
        std::fs::write(repo.path().join(".gitignore"), b"runtime/\n").unwrap();
        let runtime = repo.path().join("runtime");
        std::fs::create_dir(&runtime).unwrap();
        std::os::unix::fs::symlink("../missing-runtime", runtime.join("tool")).unwrap();

        let exclusions =
            repo_owned_git_exclusions(repo.path(), BTreeSet::from([".git".to_owned()])).unwrap();
        let links = validate_repository_tree_paths(repo.path(), &exclusions).unwrap();
        assert!(links.is_empty());

        std::os::unix::fs::symlink("missing", repo.path().join("visible-link")).unwrap();
        assert!(validate_repository_tree_paths(repo.path(), &exclusions).is_err());
    }

    #[test]
    fn repo_owned_ignores_preserve_tracked_negated_and_adjacent_paths() {
        let repo = initialized_git_repository();
        std::fs::write(
            repo.path().join(".gitignore"),
            b"runtime/*\n!runtime/keep.kt\nignored-file\n",
        )
        .unwrap();
        std::fs::create_dir(repo.path().join("runtime")).unwrap();
        std::fs::write(repo.path().join("runtime/drop.bin"), b"drop").unwrap();
        std::fs::write(repo.path().join("runtime/keep.kt"), b"fun keep() = 1\n").unwrap();
        std::fs::write(
            repo.path().join("runtime/tracked.kt"),
            b"fun tracked() = 1\n",
        )
        .unwrap();
        std::fs::write(repo.path().join("ignored-file"), b"drop").unwrap();
        std::fs::write(repo.path().join("ignored-file-neighbor"), b"keep").unwrap();
        git(
            repo.path(),
            &[
                "add",
                ".gitignore",
                "runtime/keep.kt",
                "ignored-file-neighbor",
            ],
        );
        git(repo.path(), &["add", "-f", "runtime/tracked.kt"]);

        let exclusions =
            repo_owned_git_exclusions(repo.path(), BTreeSet::from([".git".to_owned()])).unwrap();
        let (sources, _, _) = snapshot_repository(repo.path(), &exclusions).unwrap();
        let paths = sources
            .iter()
            .map(|source| source.normalized_path.as_str())
            .collect::<BTreeSet<_>>();

        assert!(paths.contains("runtime/keep.kt"));
        assert!(paths.contains("runtime/tracked.kt"));
        assert!(paths.contains("ignored-file-neighbor"));
        assert!(!paths.contains("runtime/drop.bin"));
        assert!(!paths.contains("ignored-file"));
    }

    #[test]
    fn prepared_refusal_is_exact_bound_workerless_and_replay_stable() {
        let repository = committed_git_repository();
        let authority = tempdir().unwrap();
        let (args, refusal) = prepared_refusal_args(repository.path(), authority.path());
        assert!(validate_prepared_refusal_cli(&args).unwrap());
        validate_prepared_refusal_phase(args.run_phase, args.seed_entity.as_deref()).unwrap();

        let mut first_context = RunContext::new();
        let first = match run(args, &mut first_context) {
            Ok(_) => panic!("prepared refusal must terminate before success"),
            Err(error) => error,
        };
        assert!(
            first
                .to_string()
                .contains("validated dependency-preparation refusal")
        );
        assert_eq!(first_context.stage, "DEPENDENCY_PREPARATION");
        assert_eq!(first_context.status, "REFUSED");
        assert_eq!(first_context.reason_code, refusal.reason_code);
        assert_eq!(
            first_context.detail_digest_override.as_deref(),
            Some(refusal.safe_detail_digest.as_str())
        );
        assert_eq!(first_context.provenance["workerStarted"], false);
        assert_eq!(first_context.cache["hit"], false);
        assert_eq!(first_context.telemetry.cache_hits, 0);

        let (replay_args, _) = prepared_refusal_args(repository.path(), authority.path());
        let mut replay_context = RunContext::new();
        assert!(run(replay_args, &mut replay_context).is_err());
        let first_attempt = terminal_from_context(&first_context);
        let replay_attempt = terminal_from_context(&replay_context);
        first_attempt.verify().unwrap();
        replay_attempt.verify().unwrap();
        assert_eq!(
            first_attempt.terminal_semantic_digest,
            replay_attempt.terminal_semantic_digest
        );
        assert_eq!(
            first_attempt.detail_digest,
            Some(refusal.safe_detail_digest)
        );
    }

    #[test]
    fn prepared_refusal_rejects_predecessor_series_identity() {
        let repository = committed_git_repository();
        let authority = tempdir().unwrap();
        let (args, mut refusal) = prepared_refusal_args(repository.path(), authority.path());
        refusal.series_id = "KOTLIN_REAL_REPOSITORY_K1_5_2026_08_13".to_owned();
        refusal.object_digest.clear();
        refusal.object_digest = canonical_hash(&refusal).unwrap();
        let mut raw = canonical_bytes(&refusal).unwrap();
        raw.push(b'\n');
        std::fs::write(args.prepared_refusal.as_ref().unwrap(), &raw).unwrap();
        assert!(
            consume_prepared_refusal(&args, repository.path(), &mut RunContext::new()).is_err()
        );
    }

    #[test]
    fn prepared_refusal_rejects_cross_entry_tool_source_and_file_replay() {
        let repository = committed_git_repository();
        let authority = tempdir().unwrap();
        let (mut args, refusal) = prepared_refusal_args(repository.path(), authority.path());

        args.entry_id = Some("K1-Q02".to_owned());
        assert!(
            consume_prepared_refusal(&args, repository.path(), &mut RunContext::new()).is_err()
        );
        args.entry_id = Some(refusal.entry.clone());
        args.candidate_tools_sha256 = Some(hash_bytes(b"other candidate tools"));
        assert!(
            consume_prepared_refusal(&args, repository.path(), &mut RunContext::new()).is_err()
        );
        args.candidate_tools_sha256 = Some(refusal.candidate_tools_sha256.clone());

        std::fs::write(
            repository.path().join("src/Main.kt"),
            b"fun changed() = Unit\n",
        )
        .unwrap();
        assert!(
            consume_prepared_refusal(&args, repository.path(), &mut RunContext::new()).is_err()
        );
        std::fs::write(
            repository.path().join("src/Main.kt"),
            b"fun main() = Unit\n",
        )
        .unwrap();

        let original = args.prepared_refusal.as_ref().unwrap();
        let linked_source = authority.path().join("linked-authority.json");
        std::fs::copy(original, &linked_source).unwrap();
        let linked = authority.path().join("hardlinked-refusal.json");
        std::fs::hard_link(&linked_source, &linked).unwrap();
        let linked_raw = std::fs::read(&linked).unwrap();
        args.prepared_refusal = Some(linked.canonicalize().unwrap());
        args.prepared_refusal_sha256 = Some(hash_bytes(&linked_raw));
        assert!(
            consume_prepared_refusal(&args, repository.path(), &mut RunContext::new()).is_err()
        );
    }

    #[test]
    fn prepared_refusal_rejects_noncanonical_or_mutated_authority() {
        let repository = committed_git_repository();
        let authority = tempdir().unwrap();
        let (mut args, mut refusal) = prepared_refusal_args(repository.path(), authority.path());
        refusal.reason_code = "INFRASTRUCTURE_FAILURE".to_owned();
        refusal.object_digest.clear();
        refusal.object_digest = canonical_hash(&refusal).unwrap();
        let forged = authority.path().join("FORGED_REFUSAL.json");
        let mut forged_raw = canonical_bytes(&refusal).unwrap();
        forged_raw.push(b'\n');
        std::fs::write(&forged, &forged_raw).unwrap();
        args.prepared_refusal = Some(forged.canonicalize().unwrap());
        args.prepared_refusal_sha256 = Some(hash_bytes(&forged_raw));
        assert!(
            consume_prepared_refusal(&args, repository.path(), &mut RunContext::new()).is_err()
        );

        #[cfg(unix)]
        {
            let original = authority.path().join("PREPARED_REFUSAL.json");
            let linked = authority.path().join("REFUSAL_LINK.json");
            std::os::unix::fs::symlink(&original, &linked).unwrap();
            args.prepared_refusal = Some(linked);
            args.prepared_refusal_sha256 = Some(hash_bytes(&std::fs::read(original).unwrap()));
            assert!(
                consume_prepared_refusal(&args, repository.path(), &mut RunContext::new()).is_err()
            );
        }
    }

    #[test]
    fn prepared_refusal_rejects_superseded_k1_2_series_identity() {
        let repository = committed_git_repository();
        let authority = tempdir().unwrap();
        let (_, mut refusal) = prepared_refusal_args(repository.path(), authority.path());
        refusal.series_id = "KOTLIN_REAL_REPOSITORY_K1_2_2026_08_13".to_owned();
        refusal.object_digest.clear();
        refusal.object_digest = canonical_hash(&refusal).unwrap();
        assert!(refusal.verify().is_err());
    }

    #[test]
    fn impact_normalization_covers_no_seed_budget_and_clean_seeded_paths() {
        let explicit = json!({
            "boundaryId":hash_bytes(b"explicit"),
            "kindUri":"codeclew.boundary/kotlin/explicit/1",
            "consequence":"ENUMERATION_INCOMPLETE",
        });
        let no_seed = normalized_reverse_impact(None, &[], &[], &[explicit], 2, 128).unwrap();
        assert_eq!(no_seed["status"], "UNKNOWN");
        assert_eq!(no_seed["boundaries"].as_array().unwrap().len(), 2);
        assert_closure_obligation(&no_seed);

        let entities = vec![json!({"opaqueId":"target"}), json!({"opaqueId":"caller"})];
        let facts = vec![json!({
            "factId":"call",
            "truth":"TRUE",
            "grade":"COMPILER_RESOLVED",
            "owner":"caller",
            "target":"target",
            "relation":"codeclew.relation/calls/1",
        })];
        let budget =
            normalized_reverse_impact(Some("target"), &entities, &facts, &[], 2, 1).unwrap();
        assert_eq!(budget["status"], "PARTIAL_BOUNDARY");
        assert_eq!(budget["boundaries"].as_array().unwrap().len(), 1);
        assert_closure_obligation(&budget);

        let clean =
            normalized_reverse_impact(Some("target"), &entities, &facts, &[], 2, 128).unwrap();
        assert_eq!(clean["status"], "COMPLETE_IN_SCOPE");
        assert!(clean["boundaries"].as_array().unwrap().is_empty());
        assert_eq!(clean["mandatoryObligations"].as_array().unwrap().len(), 1);
        assert_closure_obligation(&clean);
    }

    #[test]
    fn agent_projection_separates_proven_path_gaps_from_project_wide_gaps() {
        let range = json!({
            "artifactId":"source:src/Main.kt",
            "artifactContentDigest":hash_bytes(b"fun caller() = target()\n"),
            "startByte":15,
            "endByte":23,
        });
        let path_fact = json!({
            "factId":"call",
            "truth":"TRUE",
            "grade":"COMPILER_RESOLVED",
            "owner":"caller",
            "target":"target",
            "relation":"codeclew.relation/calls/1",
            "range":range,
        });
        let project_boundary = json!({
            "boundaryId":hash_bytes(b"project"),
            "kindUri":"codeclew.boundary/kotlin/unresolved-external/1",
            "consequence":"ENUMERATION_INCOMPLETE",
            "origin":null,
            "details":{"affectedRowCount":41},
        });
        let path_boundary = json!({
            "boundaryId":hash_bytes(b"path"),
            "kindUri":"codeclew.boundary/kotlin/path-local-gap/1",
            "consequence":"ENUMERATION_INCOMPLETE",
            "origin":{
                "artifactId":"source:src/Main.kt",
                "artifactContentDigest":hash_bytes(b"fun caller() = target()\n"),
                "startByte":20,
                "endByte":25,
            },
        });
        let query_specification = impact_query_specification();
        let impact = json!({
            "schema":"codeclew.impact-result/0.1",
            "status":"PARTIAL_BOUNDARY",
            "querySpecification":query_specification,
            "seedEntity":"target",
            "affected":[
                {"entityId":"target","depth":0},
                {"entityId":"caller","depth":1},
            ],
            "paths":[{"from":"target","to":"caller","factId":"call","relation":"codeclew.relation/calls/1"}],
            "boundaries":[path_boundary.clone()],
            "boundaryAssessment":{
                "querySpecificationDigest":canonical_hash(&query_specification).unwrap(),
                "projectBoundaryCount":2,
                "projectBoundarySetDigest":canonical_hash(&vec![project_boundary.clone(),path_boundary.clone()]).unwrap(),
                "queryRelevantBoundaryCount":1,
                "queryRelevantBoundarySetDigest":canonical_hash(&vec![path_boundary.clone()]).unwrap(),
            },
            "mandatoryObligations":[{
                "id":"impact-closure-completeness",
                "kind":"codeclew.obligation/impact-closure-completeness/1",
                "mandatory":true,
                "status":"UNKNOWN",
            }],
        });
        let compiler_receipt = json!({
            "method":"K2_FIR_ANALYSIS",
            "status":"ACCEPTED",
            "grade":"COMPILER_CHECKED",
            "providerPayload":{"k2Validated":true},
        });
        let selected = BTreeSet::from(["target".to_owned(), "caller".to_owned()]);
        let scoped = agent_scoped_impact(
            &impact,
            &[path_fact],
            &selected,
            &compiler_receipt,
            &[project_boundary.clone(), path_boundary.clone()],
        )
        .unwrap();
        assert_eq!(scoped["status"], "PARTIAL_BOUNDARY");
        assert_eq!(scoped["pathProof"]["status"], "SATISFIED");
        assert_eq!(scoped["boundaries"].as_array().unwrap(), &[path_boundary]);
        assert_eq!(scoped["mandatoryObligations"].as_array().unwrap().len(), 1);
        assert_eq!(scoped["projectWarnings"]["boundaryCount"], 2);
        assert_eq!(
            scoped["projectWarnings"]["coverage"],
            "FULL_PROJECT_DIGEST_AND_KIND_SUMMARY"
        );
        assert!(scoped["projectWarnings"].get("boundaries").is_none());
        assert_eq!(
            scoped["projectWarnings"]["byKind"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
    }

    #[test]
    fn generated_no_source_warning_is_retained_under_the_declared_entity_scope() {
        let applicability = boundary_applicability(
            BOUNDARY_EFFECT_OUT_OF_SCOPE,
            ["codeclew.relation/calls/1".to_owned()],
            &BTreeSet::new(),
            &BTreeSet::new(),
            false,
        );
        assert_eq!(
            applicability["entityScope"],
            Value::String(IMPACT_QUERY_ENTITY_SCOPE.to_owned())
        );
        let boundary = json!({
            "boundaryId":hash_bytes(b"generated-no-source"),
            "kindUri":"codeclew.boundary/kotlin/generated-or-no-source/1",
            "consequence":"ENUMERATION_INCOMPLETE",
            "origin":null,
            "applicability":applicability,
            "details":{"code":"GENERATED_OR_NO_SOURCE"},
        });
        let entities = vec![json!({"opaqueId":"target"})];
        let impact =
            normalized_reverse_impact(Some("target"), &entities, &[], &[boundary.clone()], 2, 128)
                .unwrap();
        assert_eq!(impact["status"], "COMPLETE_IN_SCOPE");
        assert!(impact["boundaries"].as_array().unwrap().is_empty());
        let compiler_receipt = json!({
            "method":"K2_FIR_ANALYSIS",
            "status":"ACCEPTED",
            "grade":"COMPILER_CHECKED",
            "providerPayload":{"k2Validated":true},
        });
        let scoped = agent_scoped_impact(
            &impact,
            &[],
            &BTreeSet::from(["target".to_owned()]),
            &compiler_receipt,
            &[boundary.clone()],
        )
        .unwrap();
        assert_eq!(scoped["mandatoryObligations"][0]["status"], "SATISFIED");
        assert_eq!(scoped["projectWarnings"]["boundaryCount"], 1);
        assert!(scoped["projectWarnings"].get("boundaries").is_none());
        assert_eq!(
            scoped["projectWarnings"]["byKind"][0]["kindUri"],
            boundary["kindUri"]
        );

        let mut unbound = boundary;
        unbound["applicability"]
            .as_object_mut()
            .unwrap()
            .remove("entityScope");
        let fail_closed =
            normalized_reverse_impact(Some("target"), &entities, &[], &[unbound], 2, 128).unwrap();
        assert_eq!(fail_closed["status"], "PARTIAL_BOUNDARY");
        assert_eq!(fail_closed["boundaries"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn agent_projection_rejects_an_unproven_displayed_path() {
        let fact = json!({
            "factId":"call",
            "truth":"UNKNOWN",
            "grade":"STATICALLY_APPROXIMATED",
            "owner":"caller",
            "target":"target",
            "range":{
                "artifactId":"source:src/Main.kt",
                "artifactContentDigest":hash_bytes(b"source"),
                "startByte":0,
                "endByte":1,
            },
        });
        let impact = json!({
            "paths":[{"from":"target","to":"caller","factId":"call"}],
            "boundaries":[],
            "mandatoryObligations":[],
        });
        let compiler_receipt = json!({
            "method":"K2_FIR_ANALYSIS",
            "status":"ACCEPTED",
            "grade":"COMPILER_CHECKED",
            "providerPayload":{"k2Validated":true},
        });
        let selected = BTreeSet::from(["target".to_owned(), "caller".to_owned()]);
        assert!(agent_scoped_impact(&impact, &[fact], &selected, &compiler_receipt, &[]).is_err());
    }

    #[test]
    fn compiler_boundary_classification_is_generic_and_fail_closed() {
        assert_eq!(
            compiler_boundary_effect(
                "relation",
                "K2_FIR_CFG",
                "ORDER_PROVENANCE",
                "NO_CFG_NODE_FOR_RELATION",
                false,
                true,
            ),
            BOUNDARY_EFFECT_ATTRIBUTE
        );
        assert_eq!(
            compiler_boundary_effect(
                "relation",
                "CODECLEW_RELATION_NORMALIZER",
                "OPTIONAL_RELATION_EVIDENCE",
                "ARGUMENT_MAPPING_UNAVAILABLE",
                false,
                true,
            ),
            BOUNDARY_EFFECT_ATTRIBUTE
        );
        assert_eq!(
            compiler_boundary_effect(
                "relation",
                "K2_FIR_CFG",
                "ORDER_PROVENANCE",
                "NO_CFG_NODE_FOR_RELATION",
                false,
                false,
            ),
            BOUNDARY_EFFECT_TOPOLOGY
        );
        assert_eq!(
            compiler_boundary_effect(
                "relation",
                "CODECLEW_RELATION_NORMALIZER",
                "OPTIONAL_RELATION_EVIDENCE",
                "ARGUMENT_MAPPING_UNAVAILABLE",
                false,
                false,
            ),
            BOUNDARY_EFFECT_TOPOLOGY
        );
        assert_eq!(
            compiler_boundary_effect(
                "descriptor",
                "K2_FIR",
                "DECLARATION",
                "GENERATED_OR_NO_SOURCE",
                false,
                true,
            ),
            BOUNDARY_EFFECT_OUT_OF_SCOPE
        );
        assert_eq!(
            compiler_boundary_effect(
                "relation",
                "K2_FIR",
                "ARGUMENT_MAPPING",
                "EXTERNAL_OR_LOCAL_ARGUMENT_TARGET",
                false,
                false,
            ),
            BOUNDARY_EFFECT_TOPOLOGY
        );
        assert_eq!(
            compiler_boundary_effect(
                "descriptor",
                "K2_FIR",
                "DECLARATION",
                "LOCAL_DECLARATION_UNSUPPORTED",
                false,
                true,
            ),
            BOUNDARY_EFFECT_OUT_OF_SCOPE
        );
        assert_eq!(
            compiler_boundary_effect(
                "relation",
                "K2_FIR",
                "ARGUMENT_MAPPING",
                "EXTERNAL_OR_LOCAL_ARGUMENT_TARGET",
                false,
                true,
            ),
            BOUNDARY_EFFECT_OUT_OF_SCOPE
        );
        assert_eq!(
            compiler_boundary_effect("relation", "UNKNOWN", "UNKNOWN", "NEW_CODE", false, false,),
            BOUNDARY_EFFECT_TOPOLOGY
        );
        // A compiler argument-mapping refusal currently may suppress a CALLS
        // row, so it is not safe to relabel it as an attribute-only gap.
        assert_eq!(
            compiler_boundary_effect(
                "relation",
                "K2_FIR",
                "ARGUMENT_MAPPING",
                "EXTENSION_ARGUMENT_MAPPING_UNSUPPORTED",
                false,
                true,
            ),
            BOUNDARY_EFFECT_TOPOLOGY
        );
        assert_eq!(
            compiler_boundary_effect(
                "relation",
                "K2_FIR",
                "ARGUMENT_MAPPING",
                "EXTENSION_ARGUMENT_MAPPING_UNSUPPORTED",
                true,
                true,
            ),
            BOUNDARY_EFFECT_ATTRIBUTE
        );
        assert_eq!(
            compiler_boundary_effect(
                "descriptor",
                "K2_FIR",
                "CONSTRUCTOR_DECLARATION",
                "LOCAL_CONSTRUCTOR_UNSUPPORTED",
                false,
                true,
            ),
            BOUNDARY_EFFECT_OUT_OF_SCOPE
        );
        assert_eq!(
            compiler_boundary_effect(
                "descriptor",
                "K2_FIR",
                "DECLARATION",
                "NO_COMPILER_CALLABLE_ID",
                false,
                true,
            ),
            BOUNDARY_EFFECT_TOPOLOGY
        );
    }

    fn argument_mapping_boundary() -> Value {
        json!({
            "schema":"declaration-relation-boundary/0.1",
            "file":"src/Main.kt",
            "start":10,
            "end":20,
            "owner":"p/Caller.run",
            "stage":"ARGUMENT_MAPPING",
            "code":"EXTENSION_ARGUMENT_MAPPING_UNSUPPORTED",
            "resolution":"UNKNOWN",
            "provider":"K2_FIR",
        })
    }

    fn retained_reference_witness(target: &str) -> RetainedRelationWitness {
        RetainedRelationWitness {
            kind: "REFERENCES".to_owned(),
            operation: "codeclew.relation/references/1".to_owned(),
            raw_hash: hash_bytes(format!("reference:{target}").as_bytes()),
            target_query_key: query_endpoint_key(target).unwrap(),
            resolved_owner: "callable:p/Caller.run#jvm:()V".to_owned(),
            resolved_target: format!("callable:{target}#jvm:()V"),
            target_declaration_kind: Some("FUNCTION".to_owned()),
            range: json!({
                "artifactId":"source:src/Main.kt",
                "artifactContentDigest":hash_bytes(b"source"),
                "startByte":10,
                "endByte":20,
            }),
        }
    }

    fn exact_call_topology_relation(kind: &str, target: &str) -> Value {
        json!({
            "schema":"declaration-relation/0.1",
            "file":"src/Main.kt",
            "start":10,
            "end":20,
            "owner":"p/Caller.run",
            "target":target,
            "kind":kind,
            "resolution":"PROVEN",
            "provider":"K2_FIR",
            "cfgNodeIds":[],
            "sourceProvenance":"COMPILER_UTF16_RANGE_TO_UTF8_BYTES",
            "orderProvenance":"FIR_SOURCE_RANGE",
        })
    }

    fn exact_quarantine_topology_boundary(kind: &str, target: &str) -> Value {
        json!({
            "schema":"declaration-relation-boundary/0.1",
            "file":"src/Main.kt",
            "start":10,
            "end":20,
            "owner":"p/Caller.run",
            "target":target,
            "relationKind":kind,
            "stage":"NORMALIZE",
            "code":"REFERENCE_TO_QUARANTINED_DESCRIPTOR",
            "resolution":"UNKNOWN",
            "provider":"COMPILER_RELATION_NORMALIZER",
            "rawRowHash":hash_bytes(format!("quarantined:{kind}:{target}").as_bytes()),
        })
    }

    fn exact_call_topology_owner_index(
        relations: impl IntoIterator<Item = Value>,
    ) -> ExactCallTopologyOwnerIndex {
        let mut index = ExactCallTopologyOwnerIndex::default();
        for relation in relations {
            index.index_verified_relation(&relation).unwrap();
        }
        index
    }

    #[test]
    fn exact_raw_base_call_or_constructor_makes_current_mapping_gap_attribute_only() {
        for (kind, code, expected_operation) in [
            (
                "CALLS",
                "EXTENSION_ARGUMENT_MAPPING_UNSUPPORTED",
                "codeclew.relation/calls/1",
            ),
            (
                "CONSTRUCTS",
                "VARARG_ARGUMENT_MAPPING_UNSUPPORTED",
                "codeclew.relation/constructs/1",
            ),
        ] {
            let mut boundary = argument_mapping_boundary();
            boundary["code"] = json!(code);
            let raw_target = format!("p/Raw{kind}.invoke");
            let raw_index =
                exact_call_topology_owner_index([exact_call_topology_relation(kind, &raw_target)]);
            let occurrence = source_occurrence_key(&boundary).unwrap().unwrap();
            let reference_cases = [
                BTreeMap::new(),
                BTreeMap::from([(
                    occurrence,
                    vec![
                        retained_reference_witness("p/Other.one"),
                        retained_reference_witness("p/Other.two"),
                    ],
                )]),
            ];

            for retained in reference_cases {
                // An empty materialized witness set models a raw base whose
                // endpoint is unresolved. It still prevents a duplicate
                // derived fact; the endpoint boundary carries the real gap.
                let proof = compiler_boundary_relation_proof(
                    "relation",
                    "K2_FIR",
                    "ARGUMENT_MAPPING",
                    code,
                    &boundary,
                    &raw_index,
                    &retained,
                    &hash_bytes(b"tree"),
                )
                .unwrap();
                assert!(proof.exact_topology_owner_proven);
                assert_eq!(proof.target_scope, BoundaryTargetScope::Exact);
                assert_eq!(
                    proof.target_query_key,
                    Some(query_endpoint_key(&raw_target).unwrap())
                );
                assert_eq!(proof.scoped_operation.as_deref(), Some(expected_operation));
                assert_eq!(
                    proof.retained_base_operations,
                    BTreeSet::from([expected_operation.to_owned()])
                );
                assert!(proof.derived_fact.is_none());
                assert_eq!(
                    compiler_boundary_operations_for_proof("ARGUMENT_MAPPING", code, &proof),
                    BTreeSet::from([expected_operation.to_owned()])
                );
                assert_eq!(
                    compiler_boundary_effect(
                        "relation",
                        "K2_FIR",
                        "ARGUMENT_MAPPING",
                        code,
                        true,
                        proof.source_boundary_valid,
                    ),
                    BOUNDARY_EFFECT_ATTRIBUTE
                );
            }
        }
    }

    #[test]
    fn zero_width_raw_base_is_an_exact_mapping_owner_but_reversed_range_is_not() {
        let mut boundary = argument_mapping_boundary();
        boundary["end"] = boundary["start"].clone();
        let mut relation = exact_call_topology_relation("CALLS", "p/Raw.invoke");
        relation["end"] = relation["start"].clone();

        let owners = exact_call_topology_owner_index([relation]);
        let proof = compiler_boundary_relation_proof(
            "relation",
            "K2_FIR",
            "ARGUMENT_MAPPING",
            "EXTENSION_ARGUMENT_MAPPING_UNSUPPORTED",
            &boundary,
            &owners,
            &BTreeMap::new(),
            &hash_bytes(b"tree"),
        )
        .unwrap();
        assert!(proof.exact_topology_owner_proven);
        assert_eq!(proof.target_scope, BoundaryTargetScope::Exact);
        assert_eq!(
            proof.scoped_operation.as_deref(),
            Some("codeclew.relation/calls/1")
        );
        assert!(proof.derived_fact.is_none());

        let mut reversed_boundary = boundary;
        reversed_boundary["start"] = json!(11);
        let mut reversed_relation = exact_call_topology_relation("CALLS", "p/Raw.invoke");
        reversed_relation["start"] = json!(11);
        reversed_relation["end"] = json!(10);
        assert!(source_occurrence_key(&reversed_boundary).unwrap().is_none());
        let reversed_owners = exact_call_topology_owner_index([reversed_relation]);
        let reversed_proof = compiler_boundary_relation_proof(
            "relation",
            "K2_FIR",
            "ARGUMENT_MAPPING",
            "EXTENSION_ARGUMENT_MAPPING_UNSUPPORTED",
            &reversed_boundary,
            &reversed_owners,
            &BTreeMap::new(),
            &hash_bytes(b"tree"),
        )
        .unwrap();
        assert!(!reversed_proof.exact_topology_owner_proven);
        assert_eq!(reversed_proof.target_scope, BoundaryTargetScope::Global);
        assert!(reversed_proof.scoped_operation.is_none());
        assert!(reversed_proof.retained_base_operations.is_empty());
        assert!(reversed_proof.derived_fact.is_none());
    }

    #[test]
    fn local_owner_is_a_structural_mapping_key_but_not_a_public_query_key() {
        let local_owner = "<local>/Caller.run";
        let mut boundary = argument_mapping_boundary();
        boundary["owner"] = json!(local_owner);
        let mut relation = exact_call_topology_relation("CALLS", "p/Raw.invoke");
        relation["owner"] = json!(local_owner);
        let owners = exact_call_topology_owner_index([relation]);

        let proof = compiler_boundary_relation_proof(
            "relation",
            "K2_FIR",
            "ARGUMENT_MAPPING",
            "EXTENSION_ARGUMENT_MAPPING_UNSUPPORTED",
            &boundary,
            &owners,
            &BTreeMap::new(),
            &hash_bytes(b"tree"),
        )
        .unwrap();
        assert!(proof.owner_query_key.is_none());
        assert!(proof.exact_topology_owner_proven);
        assert_eq!(proof.target_scope, BoundaryTargetScope::Exact);
        assert_eq!(
            proof.scoped_operation.as_deref(),
            Some("codeclew.relation/calls/1")
        );
        assert_eq!(
            proof.retained_base_operations,
            BTreeSet::from(["codeclew.relation/calls/1".to_owned()])
        );
        assert!(proof.derived_fact.is_none());
        assert_eq!(
            compiler_boundary_effect(
                "relation",
                "K2_FIR",
                "ARGUMENT_MAPPING",
                "EXTENSION_ARGUMENT_MAPPING_UNSUPPORTED",
                true,
                proof.source_boundary_valid,
            ),
            BOUNDARY_EFFECT_ATTRIBUTE
        );
    }

    #[test]
    fn malformed_or_ambiguous_structural_owner_stays_global() {
        for owner in [None, Some(json!("")), Some(json!(7))] {
            let mut boundary = argument_mapping_boundary();
            match owner {
                Some(owner) => boundary["owner"] = owner,
                None => {
                    boundary.as_object_mut().unwrap().remove("owner");
                }
            }
            assert!(source_occurrence_key(&boundary).unwrap().is_none());
            let proof = compiler_boundary_relation_proof(
                "relation",
                "K2_FIR",
                "ARGUMENT_MAPPING",
                "EXTENSION_ARGUMENT_MAPPING_UNSUPPORTED",
                &boundary,
                &ExactCallTopologyOwnerIndex::default(),
                &BTreeMap::new(),
                &hash_bytes(b"tree"),
            )
            .unwrap();
            assert!(!proof.exact_topology_owner_proven);
            assert_eq!(proof.target_scope, BoundaryTargetScope::Global);
            assert!(proof.scoped_operation.is_none());
            assert!(proof.retained_base_operations.is_empty());
            assert!(proof.derived_fact.is_none());
        }

        let local_owner = "<local>/Caller.run";
        let mut boundary = argument_mapping_boundary();
        boundary["owner"] = json!(local_owner);
        let mut first = exact_call_topology_relation("CALLS", "p/Raw.one");
        first["owner"] = json!(local_owner);
        let mut second = exact_call_topology_relation("CALLS", "p/Raw.two");
        second["owner"] = json!(local_owner);
        let owners = exact_call_topology_owner_index([first, second]);
        let proof = compiler_boundary_relation_proof(
            "relation",
            "K2_FIR",
            "ARGUMENT_MAPPING",
            "EXTENSION_ARGUMENT_MAPPING_UNSUPPORTED",
            &boundary,
            &owners,
            &BTreeMap::new(),
            &hash_bytes(b"tree"),
        )
        .unwrap();
        assert!(!proof.exact_topology_owner_proven);
        assert_eq!(proof.target_scope, BoundaryTargetScope::Global);
        assert!(proof.scoped_operation.is_none());
        assert!(proof.retained_base_operations.is_empty());
        assert!(proof.derived_fact.is_none());
    }

    #[test]
    fn exact_quarantine_owner_makes_current_mapping_gap_attribute_only() {
        for (kind, code, expected_operation) in [
            (
                "CALLS",
                "CONTEXT_ARGUMENT_MAPPING_UNSUPPORTED",
                "codeclew.relation/calls/1",
            ),
            (
                "CONSTRUCTS",
                "INCOMPLETE_ARGUMENT_MAPPING",
                "codeclew.relation/constructs/1",
            ),
        ] {
            let target = format!("p/Quarantined{kind}.invoke");
            let quarantine = exact_quarantine_topology_boundary(kind, &target);
            let mut owners = ExactCallTopologyOwnerIndex::default();
            owners.index_quarantine_boundary(&quarantine).unwrap();
            let mut mapping = argument_mapping_boundary();
            mapping["code"] = json!(code);
            let occurrence = source_occurrence_key(&mapping).unwrap().unwrap();
            let retained = BTreeMap::from([(
                occurrence,
                vec![
                    retained_reference_witness("p/Other.one"),
                    retained_reference_witness("p/Other.two"),
                ],
            )]);

            for references in [&BTreeMap::new(), &retained] {
                let proof = compiler_boundary_relation_proof(
                    "relation",
                    "K2_FIR",
                    "ARGUMENT_MAPPING",
                    code,
                    &mapping,
                    &owners,
                    references,
                    &hash_bytes(b"tree"),
                )
                .unwrap();
                assert!(proof.exact_topology_owner_proven);
                assert_eq!(proof.target_scope, BoundaryTargetScope::Exact);
                assert_eq!(
                    proof.target_query_key,
                    Some(query_endpoint_key(&target).unwrap())
                );
                assert_eq!(proof.scoped_operation.as_deref(), Some(expected_operation));
                assert_eq!(
                    proof.retained_base_operations,
                    BTreeSet::from([expected_operation.to_owned()])
                );
                assert!(proof.derived_fact.is_none());
                assert_eq!(
                    compiler_boundary_operations_for_proof("ARGUMENT_MAPPING", code, &proof),
                    BTreeSet::from([expected_operation.to_owned()])
                );
            }
        }
    }

    #[test]
    fn ambiguous_or_conflicting_topology_owners_never_fallback_to_references() {
        let mapping = argument_mapping_boundary();
        let occurrence = source_occurrence_key(&mapping).unwrap().unwrap();
        let references = BTreeMap::from([(
            occurrence,
            vec![retained_reference_witness("p/Legacy.invoke")],
        )]);

        let mut duplicate_quarantine = ExactCallTopologyOwnerIndex::default();
        duplicate_quarantine
            .index_quarantine_boundary(&exact_quarantine_topology_boundary(
                "CALLS",
                "p/Quarantined.one",
            ))
            .unwrap();
        duplicate_quarantine
            .index_quarantine_boundary(&exact_quarantine_topology_boundary(
                "CALLS",
                "p/Quarantined.two",
            ))
            .unwrap();

        let mut raw_and_quarantine =
            exact_call_topology_owner_index([exact_call_topology_relation(
                "CALLS",
                "p/Raw.invoke",
            )]);
        raw_and_quarantine
            .index_quarantine_boundary(&exact_quarantine_topology_boundary(
                "CALLS",
                "p/Quarantined.invoke",
            ))
            .unwrap();

        let mut conflicting_mapping = mapping.clone();
        conflicting_mapping["target"] = json!("p/Forged.invoke");
        let mut exact_quarantine = ExactCallTopologyOwnerIndex::default();
        exact_quarantine
            .index_quarantine_boundary(&exact_quarantine_topology_boundary(
                "CALLS",
                "p/Quarantined.invoke",
            ))
            .unwrap();

        let mut malformed_quarantine =
            exact_quarantine_topology_boundary("CALLS", "p/Quarantined.invoke");
        malformed_quarantine["rawRowHash"] = json!("sha256:NOT-CANONICAL");
        let mut poisoned_quarantine = ExactCallTopologyOwnerIndex::default();
        poisoned_quarantine
            .index_quarantine_boundary(&malformed_quarantine)
            .unwrap();

        for (boundary, owners) in [
            (&mapping, &duplicate_quarantine),
            (&mapping, &raw_and_quarantine),
            (&conflicting_mapping, &exact_quarantine),
            (&mapping, &poisoned_quarantine),
        ] {
            let proof = compiler_boundary_relation_proof(
                "relation",
                "K2_FIR",
                "ARGUMENT_MAPPING",
                "EXTENSION_ARGUMENT_MAPPING_UNSUPPORTED",
                boundary,
                owners,
                &references,
                &hash_bytes(b"tree"),
            )
            .unwrap();
            assert!(!proof.exact_topology_owner_proven);
            assert_eq!(proof.target_scope, BoundaryTargetScope::Global);
            assert!(proof.target_query_key.is_none());
            assert!(proof.scoped_operation.is_none());
            assert!(proof.retained_base_operations.is_empty());
            assert!(proof.derived_fact.is_none());
        }
    }

    #[test]
    fn impact_keeps_quarantine_topology_and_ignores_owned_mapping_gap() {
        let target = "p/Quarantined.invoke";
        let quarantine = exact_quarantine_topology_boundary("CALLS", target);
        let mut owners = ExactCallTopologyOwnerIndex::default();
        owners.index_quarantine_boundary(&quarantine).unwrap();
        let mapping = argument_mapping_boundary();

        let mapping_proof = compiler_boundary_relation_proof(
            "relation",
            "K2_FIR",
            "ARGUMENT_MAPPING",
            "EXTENSION_ARGUMENT_MAPPING_UNSUPPORTED",
            &mapping,
            &owners,
            &BTreeMap::new(),
            &hash_bytes(b"tree"),
        )
        .unwrap();
        let mapping_effect = compiler_boundary_effect(
            "relation",
            "K2_FIR",
            "ARGUMENT_MAPPING",
            "EXTENSION_ARGUMENT_MAPPING_UNSUPPORTED",
            !mapping_proof.retained_base_operations.is_empty(),
            mapping_proof.source_boundary_valid,
        );
        assert_eq!(mapping_effect, BOUNDARY_EFFECT_ATTRIBUTE);

        let quarantine_proof = compiler_boundary_relation_proof(
            "relation",
            "COMPILER_RELATION_NORMALIZER",
            "NORMALIZE",
            "REFERENCE_TO_QUARANTINED_DESCRIPTOR",
            &quarantine,
            &owners,
            &BTreeMap::new(),
            &hash_bytes(b"tree"),
        )
        .unwrap();
        let quarantine_effect = compiler_boundary_effect_with_partial_core(
            "relation",
            "COMPILER_RELATION_NORMALIZER",
            "NORMALIZE",
            "REFERENCE_TO_QUARANTINED_DESCRIPTOR",
            false,
            quarantine_proof.source_boundary_valid,
            quarantine_proof.target_scope,
            PartialCorePairing::NotApplicable,
        );
        assert_eq!(quarantine_effect, BOUNDARY_EFFECT_TOPOLOGY);

        let applicability = |effect: &str, proof: &CompilerBoundaryRelationProof| {
            boundary_applicability(
                effect,
                proof.scoped_operation.iter().cloned(),
                &proof.owner_query_key.iter().cloned().collect(),
                &proof.target_query_key.iter().cloned().collect(),
                proof.target_scope == BoundaryTargetScope::Exact,
            )
        };
        let boundaries = vec![
            json!({
                "boundaryId":hash_bytes(b"mapping-owned"),
                "kindUri":"codeclew.boundary/kotlin/extension-argument-mapping-unsupported/1",
                "consequence":"ENUMERATION_INCOMPLETE",
                "applicability":applicability(mapping_effect, &mapping_proof),
            }),
            json!({
                "boundaryId":hash_bytes(b"quarantine-owner"),
                "kindUri":"codeclew.boundary/kotlin/reference-to-quarantined-descriptor/1",
                "consequence":"ENUMERATION_INCOMPLETE",
                "applicability":applicability(quarantine_effect, &quarantine_proof),
            }),
        ];
        let impact = normalized_reverse_impact(
            Some(target),
            &[json!({"opaqueId":target})],
            &[],
            &boundaries,
            2,
            20,
        )
        .unwrap();
        assert_eq!(impact["status"], "PARTIAL_BOUNDARY");
        assert!(impact["paths"].as_array().unwrap().is_empty());
        let relevant = impact["boundaries"].as_array().unwrap();
        assert_eq!(relevant.len(), 1);
        assert_eq!(
            relevant[0]["kindUri"],
            "codeclew.boundary/kotlin/reference-to-quarantined-descriptor/1"
        );
    }

    #[test]
    fn ambiguous_raw_bases_do_not_fallback_to_one_reference() {
        let boundary = argument_mapping_boundary();
        let raw_index = exact_call_topology_owner_index([
            exact_call_topology_relation("CALLS", "p/Raw.one"),
            exact_call_topology_relation("CALLS", "p/Raw.two"),
        ]);
        let occurrence = source_occurrence_key(&boundary).unwrap().unwrap();
        let retained = BTreeMap::from([(
            occurrence,
            vec![retained_reference_witness("p/Legacy.invoke")],
        )]);

        let proof = compiler_boundary_relation_proof(
            "relation",
            "K2_FIR",
            "ARGUMENT_MAPPING",
            "EXTENSION_ARGUMENT_MAPPING_UNSUPPORTED",
            &boundary,
            &raw_index,
            &retained,
            &hash_bytes(b"tree"),
        )
        .unwrap();
        assert!(!proof.exact_topology_owner_proven);
        assert_eq!(proof.target_scope, BoundaryTargetScope::Global);
        assert!(proof.target_query_key.is_none());
        assert!(proof.scoped_operation.is_none());
        assert!(proof.retained_base_operations.is_empty());
        assert!(proof.derived_fact.is_none());
    }

    #[test]
    fn legacy_mapping_code_does_not_upgrade_from_raw_base() {
        let mut boundary = argument_mapping_boundary();
        boundary["code"] = json!("NO_COMPILER_CALLABLE_ID");
        let mut raw_index = exact_call_topology_owner_index([exact_call_topology_relation(
            "CALLS",
            "p/Raw.invoke",
        )]);
        raw_index
            .index_quarantine_boundary(&exact_quarantine_topology_boundary(
                "CALLS",
                "p/Quarantined.invoke",
            ))
            .unwrap();

        let proof = compiler_boundary_relation_proof(
            "relation",
            "K2_FIR",
            "ARGUMENT_MAPPING",
            "NO_COMPILER_CALLABLE_ID",
            &boundary,
            &raw_index,
            &BTreeMap::new(),
            &hash_bytes(b"tree"),
        )
        .unwrap();
        assert!(!proof.exact_topology_owner_proven);
        assert_eq!(proof.target_scope, BoundaryTargetScope::Global);
        assert!(proof.retained_base_operations.is_empty());
        assert!(proof.derived_fact.is_none());
    }

    #[test]
    fn argument_mapping_derives_call_only_from_one_exact_compiler_reference() {
        let boundary = argument_mapping_boundary();
        let occurrence = source_occurrence_key(&boundary).unwrap().unwrap();
        let target = "p/Target.invoke";
        let retained = BTreeMap::from([(occurrence, vec![retained_reference_witness(target)])]);
        let proof = compiler_boundary_relation_proof(
            "relation",
            "K2_FIR",
            "ARGUMENT_MAPPING",
            "EXTENSION_ARGUMENT_MAPPING_UNSUPPORTED",
            &boundary,
            &ExactCallTopologyOwnerIndex::default(),
            &retained,
            &hash_bytes(b"tree"),
        )
        .unwrap();

        assert_eq!(proof.target_scope, BoundaryTargetScope::Exact);
        assert_eq!(
            proof.target_query_key,
            Some(query_endpoint_key(target).unwrap())
        );
        assert_eq!(
            proof.derived_fact.as_ref().unwrap()["relation"],
            "codeclew.relation/calls/1"
        );
        assert_eq!(
            proof.retained_base_operations,
            BTreeSet::from(["codeclew.relation/calls/1".to_owned()])
        );
        assert_eq!(
            compiler_boundary_effect(
                "relation",
                "K2_FIR",
                "ARGUMENT_MAPPING",
                "EXTENSION_ARGUMENT_MAPPING_UNSUPPORTED",
                !proof.retained_base_operations.is_empty(),
                proof.source_boundary_valid,
            ),
            BOUNDARY_EFFECT_ATTRIBUTE
        );
    }

    #[test]
    fn missing_malformed_or_ambiguous_reference_stays_global_topology() {
        let boundary = argument_mapping_boundary();
        let occurrence = source_occurrence_key(&boundary).unwrap().unwrap();
        let cases = [
            BTreeMap::new(),
            BTreeMap::from([(
                occurrence.clone(),
                vec![
                    retained_reference_witness("p/Target.invoke"),
                    retained_reference_witness("p/Target.invoke"),
                ],
            )]),
        ];
        for retained in cases {
            let proof = compiler_boundary_relation_proof(
                "relation",
                "K2_FIR",
                "ARGUMENT_MAPPING",
                "EXTENSION_ARGUMENT_MAPPING_UNSUPPORTED",
                &boundary,
                &ExactCallTopologyOwnerIndex::default(),
                &retained,
                &hash_bytes(b"tree"),
            )
            .unwrap();
            assert_eq!(proof.target_scope, BoundaryTargetScope::Global);
            assert!(proof.target_query_key.is_none());
            assert!(proof.derived_fact.is_none());
            assert!(proof.retained_base_operations.is_empty());
            assert_eq!(
                compiler_boundary_effect(
                    "relation",
                    "K2_FIR",
                    "ARGUMENT_MAPPING",
                    "EXTENSION_ARGUMENT_MAPPING_UNSUPPORTED",
                    false,
                    proof.source_boundary_valid,
                ),
                BOUNDARY_EFFECT_TOPOLOGY
            );
        }

        let mut malformed = boundary;
        malformed["target"] = Value::from(7);
        let retained = BTreeMap::from([(
            occurrence,
            vec![retained_reference_witness("p/Target.invoke")],
        )]);
        let proof = compiler_boundary_relation_proof(
            "relation",
            "K2_FIR",
            "ARGUMENT_MAPPING",
            "EXTENSION_ARGUMENT_MAPPING_UNSUPPORTED",
            &malformed,
            &ExactCallTopologyOwnerIndex::default(),
            &retained,
            &hash_bytes(b"tree"),
        )
        .unwrap();
        assert_eq!(proof.target_scope, BoundaryTargetScope::Global);
        assert!(proof.derived_fact.is_none());
    }

    #[test]
    fn descriptor_symbol_identity_provides_exact_scope_but_malformed_identity_does_not() {
        let boundary = json!({
            "schema":"declaration-descriptor-boundary/0.1",
            "symbolIdentity":"callable:p/Target.run#jvm:()V",
            "stage":"DECLARATION",
            "code":"UNRESOLVED_DESCRIPTOR_TYPE",
            "resolution":"UNKNOWN",
            "provider":"K2_FIR",
        });
        let proof = compiler_boundary_relation_proof(
            "descriptor",
            "K2_FIR",
            "DECLARATION",
            "UNRESOLVED_DESCRIPTOR_TYPE",
            &boundary,
            &ExactCallTopologyOwnerIndex::default(),
            &BTreeMap::new(),
            &hash_bytes(b"tree"),
        )
        .unwrap();
        assert_eq!(proof.target_scope, BoundaryTargetScope::Exact);
        assert_eq!(
            proof.target_query_key,
            Some(query_endpoint_key("callable:p/Target.run#jvm:()V").unwrap())
        );

        let mut malformed = boundary;
        malformed["symbolIdentity"] = Value::from(7);
        let proof = compiler_boundary_relation_proof(
            "descriptor",
            "K2_FIR",
            "DECLARATION",
            "UNRESOLVED_DESCRIPTOR_TYPE",
            &malformed,
            &ExactCallTopologyOwnerIndex::default(),
            &BTreeMap::new(),
            &hash_bytes(b"tree"),
        )
        .unwrap();
        assert_eq!(proof.target_scope, BoundaryTargetScope::Global);

        let malformed_schema = json!({
            "schema":"future-descriptor-boundary/9",
            "symbolIdentity":"callable:p/Target.run#jvm:()V",
            "stage":"DECLARATION",
            "code":"UNRESOLVED_DESCRIPTOR_TYPE",
            "resolution":"UNKNOWN",
            "provider":"K2_FIR",
        });
        let proof = compiler_boundary_relation_proof(
            "descriptor",
            "K2_FIR",
            "DECLARATION",
            "UNRESOLVED_DESCRIPTOR_TYPE",
            &malformed_schema,
            &ExactCallTopologyOwnerIndex::default(),
            &BTreeMap::new(),
            &hash_bytes(b"tree"),
        )
        .unwrap();
        assert!(!proof.source_boundary_valid);
        assert_eq!(proof.target_scope, BoundaryTargetScope::Global);
    }

    fn partial_relation_row() -> Value {
        json!({
            "schema":"declaration-relation/0.1",
            "owner":"p/Caller.run",
            "target":"p/Target.run",
            "kind":"CALLS",
            "resolution":"PROVEN",
            "provider":"K2_FIR",
            "attributeCoverage":"PARTIAL",
            "sourceRowHash":hash_bytes(b"raw relation row"),
            "file":"src/Main.kt",
            "start":10,
            "end":20,
            "sourceProvenance":"COMPILER_UTF16_RANGE_TO_UTF8_BYTES",
        })
    }

    fn partial_relation_boundary(relation: &Value) -> Value {
        json!({
            "schema":"declaration-relation-boundary/0.1",
            "file":relation["file"],
            "start":relation["start"],
            "end":relation["end"],
            "owner":relation["owner"],
            "target":relation["target"],
            "relationKind":relation["kind"],
            "stage":"NORMALIZE",
            "code":"UNRESOLVED_RELATION_TYPE",
            "resolution":"UNKNOWN",
            "provider":"COMPILER_RELATION_NORMALIZER",
            "rawRowHash":relation["sourceRowHash"],
            "retainedRelationHash":canonical_hash(relation).unwrap(),
        })
    }

    fn partial_descriptor_row() -> Value {
        json!({
            "schema":"declaration-descriptor/0.1",
            "symbolIdentity":"callable:p/Target.run#jvm:()V",
            "declarationKind":"FUNCTION",
            "ownerIdentity":"class:p/Target",
            "containment":["class:p/Target"],
            "resolution":"PROVEN",
            "provider":"K2_FIR",
            "compilerCallableId":"p/Target.run",
            "attributeCoverage":"PARTIAL",
            "sourceRowHash":hash_bytes(b"raw descriptor row"),
            "file":"src/Main.kt",
            "start":30,
            "end":40,
            "sourceProvenance":"COMPILER_UTF16_RANGE_TO_UTF8_BYTES",
        })
    }

    fn partial_descriptor_boundary(descriptor: &Value) -> Value {
        json!({
            "schema":"declaration-descriptor-boundary/0.1",
            "file":"src/Main.kt",
            "symbolIdentity":"callable:p/Target.run#jvm:()V",
            "stage":"NORMALIZE",
            "code":"UNRESOLVED_DESCRIPTOR_TYPE",
            "resolution":"UNKNOWN",
            "provider":"COMPILER_DESCRIPTOR_NORMALIZER",
            "rawRowHash":descriptor["sourceRowHash"],
            "retainedDescriptorHash":canonical_hash(descriptor).unwrap(),
        })
    }

    #[test]
    fn partial_relation_boundary_is_attribute_only_after_exact_raw_core_pairing() {
        let relation = partial_relation_row();
        let boundary = partial_relation_boundary(&relation);
        let mut index = PartialCoreIndex::default();

        // A boundary cannot pair until the strict-valid raw core was indexed;
        // endpoint materialization is deliberately not part of this proof.
        assert_eq!(
            index.boundary_pairing(
                "relation",
                "COMPILER_RELATION_NORMALIZER",
                "NORMALIZE",
                "UNRESOLVED_RELATION_TYPE",
                &boundary,
            ),
            PartialCorePairing::Failed
        );
        index.index_verified_relation_core(&relation).unwrap();
        let pairing = index.boundary_pairing(
            "relation",
            "COMPILER_RELATION_NORMALIZER",
            "NORMALIZE",
            "UNRESOLVED_RELATION_TYPE",
            &boundary,
        );
        assert_eq!(pairing, PartialCorePairing::Proven);

        let mut proof = compiler_boundary_relation_proof(
            "relation",
            "COMPILER_RELATION_NORMALIZER",
            "NORMALIZE",
            "UNRESOLVED_RELATION_TYPE",
            &boundary,
            &ExactCallTopologyOwnerIndex::default(),
            &BTreeMap::new(),
            &hash_bytes(b"tree"),
        )
        .unwrap();
        apply_partial_core_pairing(&mut proof, pairing);
        assert_eq!(proof.target_scope, BoundaryTargetScope::Exact);
        assert_eq!(
            proof.target_query_key,
            Some(query_endpoint_key("p/Target.run").unwrap())
        );
        assert_eq!(
            proof.scoped_operation.as_deref(),
            Some("codeclew.relation/calls/1")
        );
        assert_eq!(
            compiler_boundary_effect_with_partial_core(
                "relation",
                "COMPILER_RELATION_NORMALIZER",
                "NORMALIZE",
                "UNRESOLVED_RELATION_TYPE",
                false,
                proof.source_boundary_valid,
                proof.target_scope,
                pairing,
            ),
            BOUNDARY_EFFECT_ATTRIBUTE
        );
    }

    #[test]
    fn zero_width_partial_core_pairs_but_reversed_range_and_forgery_fail_closed() {
        for kind in ["CALLS", "CONSTRUCTS"] {
            let mut relation = partial_relation_row();
            relation["kind"] = json!(kind);
            relation["end"] = relation["start"].clone();
            relation["sourceRowHash"] = json!(hash_bytes(format!("zero-width:{kind}").as_bytes()));
            let boundary = partial_relation_boundary(&relation);
            let mut index = PartialCoreIndex::default();
            index.index_verified_relation_core(&relation).unwrap();

            let pairing = index.boundary_pairing(
                "relation",
                "COMPILER_RELATION_NORMALIZER",
                "NORMALIZE",
                "UNRESOLVED_RELATION_TYPE",
                &boundary,
            );
            assert_eq!(pairing, PartialCorePairing::Proven);

            let mut forged = boundary.clone();
            forged["retainedRelationHash"] = json!(hash_bytes(b"forged zero-width relation"));
            assert_eq!(
                index.boundary_pairing(
                    "relation",
                    "COMPILER_RELATION_NORMALIZER",
                    "NORMALIZE",
                    "UNRESOLVED_RELATION_TYPE",
                    &forged,
                ),
                PartialCorePairing::Failed
            );

            let mut reversed = relation;
            reversed["start"] = json!(11);
            reversed["end"] = json!(10);
            reversed["sourceRowHash"] =
                json!(hash_bytes(format!("reversed-range:{kind}").as_bytes()));
            let reversed_boundary = partial_relation_boundary(&reversed);
            let mut reversed_index = PartialCoreIndex::default();
            reversed_index
                .index_verified_relation_core(&reversed)
                .unwrap();
            assert_eq!(
                reversed_index.boundary_pairing(
                    "relation",
                    "COMPILER_RELATION_NORMALIZER",
                    "NORMALIZE",
                    "UNRESOLVED_RELATION_TYPE",
                    &reversed_boundary,
                ),
                PartialCorePairing::Failed
            );
        }
    }

    #[test]
    fn partial_call_core_has_exactly_one_fact_or_endpoint_topology_owner() {
        for (kind, owner_resolved, target_resolved, expected_missing_roles) in [
            ("CALLS", false, true, vec!["OWNER"]),
            ("CONSTRUCTS", true, false, vec!["TARGET"]),
            ("CALLS", false, false, vec!["OWNER", "TARGET"]),
            ("CONSTRUCTS", true, true, vec![]),
        ] {
            let mut relation = partial_relation_row();
            relation["kind"] = json!(kind);
            relation["sourceRowHash"] = json!(hash_bytes(
                format!("partial:{kind}:{owner_resolved}:{target_resolved}").as_bytes()
            ));
            let boundary = partial_relation_boundary(&relation);
            let coverage = RelationEndpointCoverage::new(
                relation["target"].as_str().unwrap(),
                owner_resolved,
                target_resolved,
            );
            let missing_roles = ["OWNER", "TARGET"]
                .into_iter()
                .filter(|role| coverage.role_missing(role))
                .collect::<Vec<_>>();
            assert_eq!(missing_roles, expected_missing_roles);
            assert_ne!(coverage.endpoints_resolved(), !missing_roles.is_empty());

            let mut index = PartialCoreIndex::default();
            index.index_verified_relation_core(&relation).unwrap();
            let pairing = index.boundary_pairing(
                "relation",
                "COMPILER_RELATION_NORMALIZER",
                "NORMALIZE",
                "UNRESOLVED_RELATION_TYPE",
                &boundary,
            );
            assert_eq!(pairing, PartialCorePairing::Proven);

            let mut proof = compiler_boundary_relation_proof(
                "relation",
                "COMPILER_RELATION_NORMALIZER",
                "NORMALIZE",
                "UNRESOLVED_RELATION_TYPE",
                &boundary,
                &ExactCallTopologyOwnerIndex::default(),
                &BTreeMap::new(),
                &hash_bytes(b"tree"),
            )
            .unwrap();
            apply_partial_core_pairing(&mut proof, pairing);
            assert_eq!(proof.target_scope, BoundaryTargetScope::Exact);
            assert_eq!(
                proof.scoped_operation.as_deref(),
                compiler_relation_operation(kind).as_deref()
            );
            assert_eq!(
                compiler_boundary_effect_with_partial_core(
                    "relation",
                    "COMPILER_RELATION_NORMALIZER",
                    "NORMALIZE",
                    "UNRESOLVED_RELATION_TYPE",
                    false,
                    proof.source_boundary_valid,
                    proof.target_scope,
                    pairing,
                ),
                BOUNDARY_EFFECT_ATTRIBUTE
            );
        }
    }

    #[test]
    fn unresolved_non_call_type_without_retained_link_keeps_exact_topology_scope() {
        let mut boundary = partial_relation_boundary(&partial_relation_row());
        boundary["relationKind"] = json!("READS");
        boundary
            .as_object_mut()
            .unwrap()
            .remove("retainedRelationHash");
        let index = PartialCoreIndex::default();

        let pairing = index.boundary_pairing(
            "relation",
            "COMPILER_RELATION_NORMALIZER",
            "NORMALIZE",
            "UNRESOLVED_RELATION_TYPE",
            &boundary,
        );
        assert_eq!(pairing, PartialCorePairing::NotApplicable);

        let mut proof = compiler_boundary_relation_proof(
            "relation",
            "COMPILER_RELATION_NORMALIZER",
            "NORMALIZE",
            "UNRESOLVED_RELATION_TYPE",
            &boundary,
            &ExactCallTopologyOwnerIndex::default(),
            &BTreeMap::new(),
            &hash_bytes(b"tree"),
        )
        .unwrap();
        apply_partial_core_pairing(&mut proof, pairing);
        assert_eq!(proof.target_scope, BoundaryTargetScope::Exact);
        assert_eq!(
            proof.target_query_key,
            Some(query_endpoint_key("p/Target.run").unwrap())
        );
        assert_eq!(
            proof.scoped_operation.as_deref(),
            Some("codeclew.relation/reads/1")
        );
        assert_eq!(
            compiler_boundary_effect_with_partial_core(
                "relation",
                "COMPILER_RELATION_NORMALIZER",
                "NORMALIZE",
                "UNRESOLVED_RELATION_TYPE",
                false,
                proof.source_boundary_valid,
                proof.target_scope,
                pairing,
            ),
            BOUNDARY_EFFECT_TOPOLOGY
        );
    }

    #[test]
    fn local_owner_non_call_type_is_exact_and_irrelevant_to_reverse_call_impact() {
        let target = "p/Target.run";
        let mut boundary = partial_relation_boundary(&partial_relation_row());
        boundary["owner"] = json!("<local>/Caller.run");
        boundary["relationKind"] = json!("READS");
        boundary
            .as_object_mut()
            .unwrap()
            .remove("retainedRelationHash");
        let pairing = PartialCoreIndex::default().boundary_pairing(
            "relation",
            "COMPILER_RELATION_NORMALIZER",
            "NORMALIZE",
            "UNRESOLVED_RELATION_TYPE",
            &boundary,
        );
        assert_eq!(pairing, PartialCorePairing::NotApplicable);

        let proof = compiler_boundary_relation_proof(
            "relation",
            "COMPILER_RELATION_NORMALIZER",
            "NORMALIZE",
            "UNRESOLVED_RELATION_TYPE",
            &boundary,
            &ExactCallTopologyOwnerIndex::default(),
            &BTreeMap::new(),
            &hash_bytes(b"tree"),
        )
        .unwrap();
        assert!(proof.owner_query_key.is_none());
        assert_eq!(proof.target_scope, BoundaryTargetScope::Exact);
        assert_eq!(
            proof.target_query_key,
            Some(query_endpoint_key(target).unwrap())
        );
        assert_eq!(
            proof.scoped_operation.as_deref(),
            Some("codeclew.relation/reads/1")
        );
        let effect = compiler_boundary_effect_with_partial_core(
            "relation",
            "COMPILER_RELATION_NORMALIZER",
            "NORMALIZE",
            "UNRESOLVED_RELATION_TYPE",
            false,
            proof.source_boundary_valid,
            proof.target_scope,
            pairing,
        );
        assert_eq!(effect, BOUNDARY_EFFECT_TOPOLOGY);

        let applicability = boundary_applicability(
            effect,
            proof.scoped_operation.iter().cloned(),
            &proof.owner_query_key.iter().cloned().collect(),
            &proof.target_query_key.iter().cloned().collect(),
            true,
        );
        let impact = normalized_reverse_impact(
            Some(target),
            &[json!({"opaqueId":target})],
            &[],
            &[json!({
                "boundaryId":hash_bytes(b"local-owner-read-type"),
                "kindUri":"codeclew.boundary/kotlin/unresolved-relation-type/1",
                "consequence":"ENUMERATION_INCOMPLETE",
                "applicability":applicability,
            })],
            2,
            20,
        )
        .unwrap();
        assert_eq!(impact["status"], "COMPLETE_IN_SCOPE");
        assert!(impact["boundaries"].as_array().unwrap().is_empty());
        assert_eq!(
            impact["boundaryAssessment"]["queryRelevantBoundaryCount"],
            0
        );
    }

    #[test]
    fn valid_non_call_type_with_external_target_is_out_of_scope_with_actual_operation() {
        for target in ["null", "<local-function>"] {
            let mut boundary = partial_relation_boundary(&partial_relation_row());
            boundary["target"] = json!(target);
            boundary["relationKind"] = json!("READS");
            boundary
                .as_object_mut()
                .unwrap()
                .remove("retainedRelationHash");
            let index = PartialCoreIndex::default();
            let pairing = index.boundary_pairing(
                "relation",
                "COMPILER_RELATION_NORMALIZER",
                "NORMALIZE",
                "UNRESOLVED_RELATION_TYPE",
                &boundary,
            );
            assert_eq!(pairing, PartialCorePairing::NotApplicable);

            let mut proof = compiler_boundary_relation_proof(
                "relation",
                "COMPILER_RELATION_NORMALIZER",
                "NORMALIZE",
                "UNRESOLVED_RELATION_TYPE",
                &boundary,
                &ExactCallTopologyOwnerIndex::default(),
                &BTreeMap::new(),
                &hash_bytes(b"tree"),
            )
            .unwrap();
            apply_partial_core_pairing(&mut proof, pairing);
            assert_eq!(proof.target_scope, BoundaryTargetScope::OutOfScope);
            assert!(proof.target_query_key.is_none());
            assert_eq!(
                proof.scoped_operation.as_deref(),
                Some("codeclew.relation/reads/1")
            );
            let effect = compiler_boundary_effect_with_partial_core(
                "relation",
                "COMPILER_RELATION_NORMALIZER",
                "NORMALIZE",
                "UNRESOLVED_RELATION_TYPE",
                false,
                proof.source_boundary_valid,
                proof.target_scope,
                pairing,
            );
            assert_eq!(effect, BOUNDARY_EFFECT_OUT_OF_SCOPE);
            let applicability = boundary_applicability(
                effect,
                proof.scoped_operation.iter().cloned(),
                &proof.owner_query_key.iter().cloned().collect(),
                &BTreeSet::new(),
                false,
            );
            assert_eq!(
                applicability["operations"],
                json!(["codeclew.relation/reads/1"])
            );
            assert_eq!(applicability["entityScope"], IMPACT_QUERY_ENTITY_SCOPE);
        }

        let mut malformed = partial_relation_boundary(&partial_relation_row());
        malformed["target"] = json!("null");
        malformed["relationKind"] = json!(7);
        malformed
            .as_object_mut()
            .unwrap()
            .remove("retainedRelationHash");
        let index = PartialCoreIndex::default();
        let pairing = index.boundary_pairing(
            "relation",
            "COMPILER_RELATION_NORMALIZER",
            "NORMALIZE",
            "UNRESOLVED_RELATION_TYPE",
            &malformed,
        );
        assert_eq!(pairing, PartialCorePairing::Failed);
        let mut proof = compiler_boundary_relation_proof(
            "relation",
            "COMPILER_RELATION_NORMALIZER",
            "NORMALIZE",
            "UNRESOLVED_RELATION_TYPE",
            &malformed,
            &ExactCallTopologyOwnerIndex::default(),
            &BTreeMap::new(),
            &hash_bytes(b"tree"),
        )
        .unwrap();
        apply_partial_core_pairing(&mut proof, pairing);
        assert_eq!(proof.target_scope, BoundaryTargetScope::Global);
        assert!(proof.scoped_operation.is_none());
        assert_eq!(
            compiler_boundary_effect_with_partial_core(
                "relation",
                "COMPILER_RELATION_NORMALIZER",
                "NORMALIZE",
                "UNRESOLVED_RELATION_TYPE",
                false,
                proof.source_boundary_valid,
                proof.target_scope,
                pairing,
            ),
            BOUNDARY_EFFECT_TOPOLOGY
        );
    }

    #[test]
    fn unresolved_call_type_without_retained_link_fails_closed() {
        let mut boundary = partial_relation_boundary(&partial_relation_row());
        boundary
            .as_object_mut()
            .unwrap()
            .remove("retainedRelationHash");
        let index = PartialCoreIndex::default();

        let pairing = index.boundary_pairing(
            "relation",
            "COMPILER_RELATION_NORMALIZER",
            "NORMALIZE",
            "UNRESOLVED_RELATION_TYPE",
            &boundary,
        );
        assert_eq!(pairing, PartialCorePairing::Failed);

        let mut proof = compiler_boundary_relation_proof(
            "relation",
            "COMPILER_RELATION_NORMALIZER",
            "NORMALIZE",
            "UNRESOLVED_RELATION_TYPE",
            &boundary,
            &ExactCallTopologyOwnerIndex::default(),
            &BTreeMap::new(),
            &hash_bytes(b"tree"),
        )
        .unwrap();
        apply_partial_core_pairing(&mut proof, pairing);
        assert_eq!(proof.target_scope, BoundaryTargetScope::Global);
        assert!(proof.target_query_key.is_none());
        assert!(proof.scoped_operation.is_none());
    }

    #[test]
    fn non_call_retained_link_claim_and_malformed_kind_fail_closed() {
        let mut forged_non_call = partial_relation_boundary(&partial_relation_row());
        forged_non_call["relationKind"] = json!("READS");
        forged_non_call["retainedRelationHash"] = json!(hash_bytes(b"forged retained row"));
        let mut malformed_kind = forged_non_call.clone();
        malformed_kind
            .as_object_mut()
            .unwrap()
            .remove("retainedRelationHash");
        malformed_kind["relationKind"] = json!(7);
        let index = PartialCoreIndex::default();

        for boundary in [forged_non_call, malformed_kind] {
            let pairing = index.boundary_pairing(
                "relation",
                "COMPILER_RELATION_NORMALIZER",
                "NORMALIZE",
                "UNRESOLVED_RELATION_TYPE",
                &boundary,
            );
            assert_eq!(pairing, PartialCorePairing::Failed);

            let mut proof = compiler_boundary_relation_proof(
                "relation",
                "COMPILER_RELATION_NORMALIZER",
                "NORMALIZE",
                "UNRESOLVED_RELATION_TYPE",
                &boundary,
                &ExactCallTopologyOwnerIndex::default(),
                &BTreeMap::new(),
                &hash_bytes(b"tree"),
            )
            .unwrap();
            apply_partial_core_pairing(&mut proof, pairing);
            assert_eq!(proof.target_scope, BoundaryTargetScope::Global);
            assert!(proof.target_query_key.is_none());
            assert!(proof.scoped_operation.is_none());
        }
    }

    #[test]
    fn partial_relation_pairing_fails_closed_for_missing_forged_or_mismatched_links() {
        let relation = partial_relation_row();
        let boundary = partial_relation_boundary(&relation);
        let mut index = PartialCoreIndex::default();
        index.index_verified_relation_core(&relation).unwrap();

        let mut missing_retained = boundary.clone();
        missing_retained
            .as_object_mut()
            .unwrap()
            .remove("retainedRelationHash");
        let mut forged_retained = boundary.clone();
        forged_retained["retainedRelationHash"] = json!(hash_bytes(b"forged retained row"));
        let mut mismatched_source = boundary.clone();
        mismatched_source["rawRowHash"] = json!(hash_bytes(b"different raw row"));
        let mut mismatched_core = boundary.clone();
        mismatched_core["target"] = json!("p/Other.run");
        let mut malformed_digest = boundary.clone();
        malformed_digest["rawRowHash"] = json!("sha256:NOT-LOWERCASE");

        for malformed in [
            missing_retained,
            forged_retained,
            mismatched_source,
            mismatched_core,
            malformed_digest,
        ] {
            let pairing = index.boundary_pairing(
                "relation",
                "COMPILER_RELATION_NORMALIZER",
                "NORMALIZE",
                "UNRESOLVED_RELATION_TYPE",
                &malformed,
            );
            assert_eq!(pairing, PartialCorePairing::Failed);
            let mut proof = compiler_boundary_relation_proof(
                "relation",
                "COMPILER_RELATION_NORMALIZER",
                "NORMALIZE",
                "UNRESOLVED_RELATION_TYPE",
                &malformed,
                &ExactCallTopologyOwnerIndex::default(),
                &BTreeMap::new(),
                &hash_bytes(b"tree"),
            )
            .unwrap();
            apply_partial_core_pairing(&mut proof, pairing);
            assert_eq!(proof.target_scope, BoundaryTargetScope::Global);
            assert!(proof.target_query_key.is_none());
            assert!(proof.scoped_operation.is_none());
            assert_eq!(
                compiler_boundary_effect_with_partial_core(
                    "relation",
                    "COMPILER_RELATION_NORMALIZER",
                    "NORMALIZE",
                    "UNRESOLVED_RELATION_TYPE",
                    false,
                    proof.source_boundary_valid,
                    proof.target_scope,
                    pairing,
                ),
                BOUNDARY_EFFECT_TOPOLOGY
            );
        }
    }

    #[test]
    fn partial_descriptor_pairing_is_exact_and_quarantined_references_never_upgrade() {
        let descriptor = partial_descriptor_row();
        let boundary = partial_descriptor_boundary(&descriptor);
        let mut index = PartialCoreIndex::default();
        index.index_emitted_descriptor(&descriptor).unwrap();

        let pairing = index.boundary_pairing(
            "descriptor",
            "COMPILER_DESCRIPTOR_NORMALIZER",
            "NORMALIZE",
            "UNRESOLVED_DESCRIPTOR_TYPE",
            &boundary,
        );
        assert_eq!(pairing, PartialCorePairing::Proven);
        assert_eq!(
            compiler_boundary_effect_with_partial_core(
                "descriptor",
                "COMPILER_DESCRIPTOR_NORMALIZER",
                "NORMALIZE",
                "UNRESOLVED_DESCRIPTOR_TYPE",
                false,
                true,
                BoundaryTargetScope::Exact,
                pairing,
            ),
            BOUNDARY_EFFECT_ATTRIBUTE
        );

        let mut forged = boundary.clone();
        forged["retainedDescriptorHash"] = json!(hash_bytes(b"not the emitted descriptor"));
        assert_eq!(
            index.boundary_pairing(
                "descriptor",
                "COMPILER_DESCRIPTOR_NORMALIZER",
                "NORMALIZE",
                "UNRESOLVED_DESCRIPTOR_TYPE",
                &forged,
            ),
            PartialCorePairing::Failed
        );

        let mut quarantined = partial_relation_boundary(&partial_relation_row());
        quarantined["code"] = json!("REFERENCE_TO_QUARANTINED_DESCRIPTOR");
        assert_eq!(
            index.boundary_pairing(
                "relation",
                "COMPILER_RELATION_NORMALIZER",
                "NORMALIZE",
                "REFERENCE_TO_QUARANTINED_DESCRIPTOR",
                &quarantined,
            ),
            PartialCorePairing::NotApplicable
        );
        assert_eq!(
            compiler_boundary_effect_with_partial_core(
                "relation",
                "COMPILER_RELATION_NORMALIZER",
                "NORMALIZE",
                "REFERENCE_TO_QUARANTINED_DESCRIPTOR",
                false,
                true,
                BoundaryTargetScope::Exact,
                PartialCorePairing::NotApplicable,
            ),
            BOUNDARY_EFFECT_TOPOLOGY
        );
    }

    #[test]
    fn argument_mapping_is_attribute_only_only_after_base_call_survives() {
        let entities = vec![json!({"opaqueId":"target"}), json!({"opaqueId":"caller"})];
        let facts = vec![json!({
            "factId":"call",
            "truth":"TRUE",
            "grade":"COMPILER_RESOLVED",
            "owner":"caller",
            "target":"target",
            "relation":"codeclew.relation/calls/1",
        })];
        let boundary = |effect: &str| {
            json!({
                "boundaryId":hash_bytes(effect.as_bytes()),
                "kindUri":"codeclew.boundary/kotlin/argument-mapping/1",
                "consequence":"ENUMERATION_INCOMPLETE",
                "applicability":boundary_applicability(
                    effect,
                    ["codeclew.relation/calls/1".to_owned()],
                    &BTreeSet::new(),
                    &BTreeSet::new(),
                    false,
                ),
            })
        };
        let retained = normalized_reverse_impact(
            Some("target"),
            &entities,
            &facts,
            &[boundary(BOUNDARY_EFFECT_ATTRIBUTE)],
            2,
            20,
        )
        .unwrap();
        assert_eq!(retained["status"], "COMPLETE_IN_SCOPE");
        assert_eq!(retained["paths"].as_array().unwrap().len(), 1);

        let suppressed = normalized_reverse_impact(
            Some("target"),
            &entities,
            &[],
            &[boundary(BOUNDARY_EFFECT_TOPOLOGY)],
            2,
            20,
        )
        .unwrap();
        assert_eq!(suppressed["status"], "PARTIAL_BOUNDARY");
        assert_eq!(suppressed["boundaries"].as_array().unwrap().len(), 1);
    }

    fn syntax_fallback_args() -> Args {
        Args {
            repo: PathBuf::from("/unused"),
            compilation: ":/main".to_owned(),
            seed_entity: Some("example.Seed".to_owned()),
            max_depth: 2,
            max_entities: 128,
            agent_output: true,
            state_root: None,
            build_state_root: None,
            attempt_output: None,
            run_phase: RunPhase::Cold,
            prepared_refusal: None,
            prepared_refusal_sha256: None,
            entry_id: None,
            candidate_tools_sha256: None,
            build_input_digest: None,
            preparation_receipt_digest: None,
        }
    }

    #[test]
    fn source_syntax_fallback_is_allowlisted_and_excludes_integrity_failures() {
        for code in [
            ErrorCode::UnsupportedKotlinVersion,
            ErrorCode::UnsupportedCompilerPluginAbi,
            ErrorCode::UnsupportedProjectConfiguration,
            ErrorCode::WorkerPreparationRequired,
            ErrorCode::IncompleteSemanticAnalysis,
        ] {
            assert!(source_syntax_fallback_error(&ClewError::new(
                code,
                "diagnostic text is deliberately not authority"
            )));
        }
        for code in [
            ErrorCode::ProjectModelChanged,
            ErrorCode::WorkerProtocolMismatch,
            ErrorCode::WorkerCrashed,
            ErrorCode::InvalidInput,
            ErrorCode::Internal,
        ] {
            assert!(!source_syntax_fallback_error(&ClewError::new(
                code,
                "must remain fatal"
            )));
        }
    }

    #[test]
    fn source_syntax_fallback_rechecks_the_exact_snapshot_and_rejects_mutation() {
        let repository = committed_git_repository();
        let repo = repository.path().canonicalize().unwrap();
        let ignored_components = [
            ".git",
            ".gradle",
            ".kotlin",
            ".semantic-thread",
            "build",
            "target",
            "node_modules",
            ".idea",
            ".vscode",
        ]
        .into_iter()
        .map(ToOwned::to_owned)
        .collect::<BTreeSet<_>>();
        let exclusions = repository_exclusions(&repo, &ignored_components);
        let tracked_symlinks = validate_repository_tree_paths(&repo, &exclusions).unwrap();
        let (sources, _, _) = snapshot_repository(&repo, &exclusions).unwrap();
        let tree_digest = k1_repository_tree_digest(&sources, &tracked_symlinks).unwrap();
        let mut vcs_revision = git_revision(&repo).unwrap();
        let dirty = git_dirty(&repo).unwrap();
        let git_status = git_status_digest(&repo).unwrap();
        verify_source_syntax_snapshot_unchanged(
            &repo,
            &ignored_components,
            &sources,
            &tree_digest,
            vcs_revision.as_deref(),
            dirty,
            git_status.as_deref(),
            &tracked_symlinks,
        )
        .unwrap();

        git(
            &repo,
            &[
                "-c",
                "user.name=Codeclew Test",
                "-c",
                "user.email=codeclew@example.invalid",
                "commit",
                "--allow-empty",
                "-q",
                "-m",
                "move revision",
            ],
        );
        let revision_error = verify_source_syntax_snapshot_unchanged(
            &repo,
            &ignored_components,
            &sources,
            &tree_digest,
            vcs_revision.as_deref(),
            dirty,
            git_status.as_deref(),
            &tracked_symlinks,
        )
        .unwrap_err();
        assert!(
            revision_error
                .to_string()
                .contains("revision or dirty state changed")
        );
        vcs_revision = git_revision(&repo).unwrap();
        verify_source_syntax_snapshot_unchanged(
            &repo,
            &ignored_components,
            &sources,
            &tree_digest,
            vcs_revision.as_deref(),
            dirty,
            git_status.as_deref(),
            &tracked_symlinks,
        )
        .unwrap();

        std::fs::write(
            repo.join("src/Main.kt"),
            b"fun main() = error(\"changed\")\n",
        )
        .unwrap();
        let error = verify_source_syntax_snapshot_unchanged(
            &repo,
            &ignored_components,
            &sources,
            &tree_digest,
            vcs_revision.as_deref(),
            dirty,
            git_status.as_deref(),
            &tracked_symlinks,
        )
        .unwrap_err();
        assert!(error.to_string().contains("snapshot changed"));
    }

    #[test]
    fn main_kotlin_file_selection_is_exact_and_excludes_tests_and_kts_build_scripts() {
        let source = |path: &str| SourceInput {
            artifact_id: hash_bytes(path.as_bytes()),
            normalized_path: path.to_owned(),
            content_digest: hash_bytes(b"source"),
            size_bytes: 6,
            origin: "SOURCE".to_owned(),
        };
        let selected = main_kotlin_source_files(&[
            source("src/main/kotlin/example/App.kt"),
            source("feature/src/main/kotlin/example/Feature.kt"),
            source("src/test/kotlin/example/AppTest.kt"),
            source("buildSrc/src/main/kotlin/BuildLogic.kt"),
            source("settings.gradle.kts"),
            source("src/main/java/example/JavaMain.kt"),
        ]);
        assert_eq!(
            selected,
            vec![
                "buildSrc/src/main/kotlin/BuildLogic.kt".to_owned(),
                "feature/src/main/kotlin/example/Feature.kt".to_owned(),
                "src/main/kotlin/example/App.kt".to_owned(),
            ]
        );
    }

    #[test]
    fn source_syntax_projection_is_useful_partial_without_semantic_claims_or_diagnostics() {
        let path = "feature/src/main/kotlin/example/App.kt";
        let source = SourceInput {
            artifact_id: hash_bytes(path.as_bytes()),
            normalized_path: path.to_owned(),
            content_digest: hash_bytes(b"package example\nfun app() = Unit\n"),
            size_bytes: 33,
            origin: "SOURCE".to_owned(),
        };
        let facts = json!({
            "analysisMode":"SYNTAX_DECLARATIONS",
            "k2Validated":false,
            "partial":true,
            "files":[{
                "path":path,
                "contentHash":source.content_digest,
                "package":"example",
                "imports":["kotlin.Unit"],
                "declarations":[{
                    "declarationId":"declaration:app",
                    "symbolId":"source-declared:example.app",
                    "symbolIdentity":{"package":"example","jvmDescriptor":"()V"},
                    "jvmDescriptor":"()V",
                    "kind":"KtNamedFunction",
                    "name":"app",
                    "sourceOrigin":{"file":path,"rangeStart":16,"rangeEnd":32},
                }],
                "diagnostics":[{
                    "message":"compiler failed below /workspace/user/project and must stay private"
                }],
            }],
            "diagnostics":[{
                "message":"raw diagnostic /workspace/user/toolchain"
            }],
        });
        let failure = ClewError::new(
            ErrorCode::UnsupportedProjectConfiguration,
            "Gradle failed in /workspace/user/project",
        );
        let projection = source_syntax_agent_projection(
            &syntax_fallback_args(),
            &facts,
            std::slice::from_ref(&source),
            &[path.to_owned()],
            &hash_bytes(b"adapter"),
            &hash_bytes(b"tree"),
            Some("revision"),
            true,
            "BUILD_DISCOVERY",
            &failure,
            123,
            source.size_bytes,
            456,
        )
        .unwrap();

        assert_eq!(
            projection
                .pointer("/adapter/adapterId")
                .and_then(Value::as_str),
            Some("codeclew.kotlin-source-syntax")
        );
        assert_eq!(
            projection
                .pointer("/compiler/method")
                .and_then(Value::as_str),
            Some("SOURCE_SYNTAX_DECLARATIONS")
        );
        assert_eq!(
            projection
                .pointer("/compiler/k2Validated")
                .and_then(Value::as_bool),
            Some(false)
        );
        assert!(
            projection
                .pointer("/semanticFacts")
                .and_then(Value::as_array)
                .unwrap()
                .is_empty()
        );
        assert!(
            projection
                .pointer("/pathFacts")
                .and_then(Value::as_array)
                .unwrap()
                .is_empty()
        );
        assert!(
            projection
                .pointer("/impact/paths")
                .and_then(Value::as_array)
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            projection
                .pointer("/impact/pathProof/status")
                .and_then(Value::as_str),
            Some("SATISFIED")
        );
        let boundaries = projection
            .pointer("/impact/boundaries")
            .and_then(Value::as_array)
            .unwrap();
        assert_eq!(boundaries.len(), 1);
        assert_eq!(
            boundaries[0]
                .pointer("/applicability/locality")
                .and_then(Value::as_str),
            Some("GLOBAL")
        );
        assert_eq!(
            boundaries[0].get("kindUri").and_then(Value::as_str),
            Some("codeclew.boundary/kotlin/toolchain-or-semantic-unavailable/1")
        );
        assert_eq!(
            boundaries[0].get("consequence").and_then(Value::as_str),
            Some("SEMANTIC_TOPOLOGY_AND_COMPILATION_MEMBERSHIP_UNKNOWN")
        );
        let obligations = projection
            .pointer("/impact/mandatoryObligations")
            .and_then(Value::as_array)
            .unwrap();
        assert_eq!(obligations.len(), 1);
        assert_eq!(
            obligations[0].get("mandatory").and_then(Value::as_bool),
            Some(true)
        );
        assert_eq!(
            obligations[0].get("status").and_then(Value::as_str),
            Some("UNKNOWN")
        );
        assert_eq!(
            projection
                .pointer("/syntax/repositoryMainKotlinCandidates/0/declarations/0/name")
                .and_then(Value::as_str),
            Some("app")
        );
        assert_eq!(
            projection
                .pointer("/syntax/repositoryMainKotlinCandidates/0/imports/0")
                .and_then(Value::as_str),
            Some("kotlin.Unit")
        );
        assert_eq!(
            projection
                .pointer("/syntax/compilationMembership")
                .and_then(Value::as_str),
            Some("UNKNOWN")
        );
        assert_eq!(
            projection
                .pointer("/syntax/coverage")
                .and_then(Value::as_str),
            Some("EXACT_REPOSITORY_MAIN_KOTLIN_CANDIDATES")
        );
        assert_eq!(
            projection
                .pointer("/projectionAuthority/semanticCacheReuse")
                .and_then(Value::as_str),
            Some("FORBIDDEN")
        );
        let encoded = String::from_utf8(canonical_bytes(&projection).unwrap()).unwrap();
        assert!(!encoded.contains("/workspace/user"));
        assert!(!encoded.contains("raw diagnostic"));
        assert!(!encoded.contains("compiler failed below"));
        assert!(!encoded.contains("source-declared:example.app"));
        assert!(!encoded.contains("jvmDescriptor"));
    }
}
