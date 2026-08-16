use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fs::File;
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::process::Command;
use std::time::Instant;
use walkdir::WalkDir;

mod core_bridge;

pub use core_bridge::{CoreBindingSummary, validate_core_binding};

pub const ADAPTER_OUTPUT_SCHEMA: &str = "codeclew.adapter-output/0.1";
pub const IMPACT_CLOSURE_SPEC: &str = "codeclew.impact.reverse-call-graph/0.1";
pub const BOUNDARY_EFFECT_TOPOLOGY: &str = "TOPOLOGY_ENUMERATION";
pub const BOUNDARY_EFFECT_ATTRIBUTE: &str = "ATTRIBUTE_ONLY";
pub const BOUNDARY_EFFECT_BUILD_FIDELITY: &str = "BUILD_FIDELITY";
pub const BOUNDARY_EFFECT_OUT_OF_SCOPE: &str = "OUT_OF_SCOPE";
pub const IMPACT_QUERY_DIRECTION: &str = "INCOMING";
pub const IMPACT_QUERY_ENTITY_SCOPE: &str = "SOURCE_DEFINED_NON_LOCAL_RESOLVED";
pub const IMPACT_QUERY_OPERATIONS: &[&str] = &[
    "codeclew.relation/calls/1",
    "codeclew.relation/constructs/1",
];

pub fn impact_query_specification() -> Value {
    serde_json::json!({
        "schema":"codeclew.impact-query-specification/0.1",
        "direction":IMPACT_QUERY_DIRECTION,
        "entityScope":IMPACT_QUERY_ENTITY_SCOPE,
        "operations":IMPACT_QUERY_OPERATIONS,
    })
}

/// Produces the opaque, canonical key used to compare a provider endpoint
/// identity with an entity in a bounded impact query. The original compiler
/// identity does not have to be copied into an aggregated boundary.
pub fn query_endpoint_key(identity: &str) -> Result<String> {
    canonical_hash(&serde_json::json!({
        "schema":"codeclew.query-endpoint-key/0.1",
        "identity":identity,
    }))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AdapterIdentity {
    pub adapter_id: String,
    pub version: String,
    pub binary_digest: String,
    pub language_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SourceInput {
    pub artifact_id: String,
    pub normalized_path: String,
    pub content_digest: String,
    pub size_bytes: u64,
    pub origin: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SnapshotInput {
    pub repository_tree_digest: String,
    pub vcs_revision: Option<String>,
    pub dirty: bool,
    pub sources: Vec<SourceInput>,
    pub build_system_uri: String,
    pub build_model_digest: String,
    pub build_configuration_digest: String,
    pub dependency_graph_digest: String,
    pub toolchain: Value,
    pub targets: Vec<Value>,
    pub relevant_environment: Vec<Value>,
    pub generated_sources_manifest_digest: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CostRecord {
    pub total_wall_micros: u64,
    pub repository_snapshot_micros: u64,
    pub build_discovery_micros: u64,
    pub cold_index_micros: u64,
    pub warm_index_micros: u64,
    pub adapter_micros: u64,
    pub query_micros: u64,
    pub source_bytes_read: u64,
    pub emitted_bytes: u64,
    pub stored_fact_bytes: u64,
    pub model_visible_source_bytes: u64,
    pub cache_requests: u64,
    pub cache_hits: u64,
    pub provider_processing_micros: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AdapterOutput {
    pub schema: String,
    pub adapter: AdapterIdentity,
    pub snapshot_input: SnapshotInput,
    pub capability_descriptors: Vec<Value>,
    pub entities: Vec<Value>,
    pub occurrences: Vec<Value>,
    pub facts: Vec<Value>,
    pub boundaries: Vec<Value>,
    pub compiler_receipt: Value,
    pub impact: Value,
    pub cost: CostRecord,
    pub output_digest: String,
}

impl AdapterOutput {
    pub fn seal(&mut self) -> Result<()> {
        self.output_digest = format!("sha256:{}", "0".repeat(64));
        loop {
            let length = canonical_bytes(self)?.len() as u64;
            if self.cost.emitted_bytes == length {
                break;
            }
            self.cost.emitted_bytes = length;
        }
        self.output_digest.clear();
        self.output_digest = canonical_hash(self)?;
        Ok(())
    }

    pub fn verify_seal(&self) -> Result<()> {
        let expected = self.output_digest.clone();
        if self.cost.emitted_bytes != canonical_bytes(self)?.len() as u64 {
            bail!("adapter output emittedBytes does not match canonical output size");
        }
        let mut projection = self.clone();
        projection.output_digest.clear();
        if canonical_hash(&projection)? != expected {
            bail!("adapter output digest mismatch");
        }
        Ok(())
    }
}

pub fn canonical_value<T: Serialize>(input: &T) -> Result<Value> {
    Ok(sort_value(serde_json::to_value(input)?))
}

pub fn canonical_bytes<T: Serialize>(input: &T) -> Result<Vec<u8>> {
    Ok(serde_json::to_vec(&canonical_value(input)?)?)
}

pub fn canonical_hash<T: Serialize>(input: &T) -> Result<String> {
    Ok(hash_bytes(&canonical_bytes(input)?))
}

pub fn hash_bytes(input: &[u8]) -> String {
    format!("sha256:{}", hex::encode(Sha256::digest(input)))
}

fn sort_value(value: Value) -> Value {
    match value {
        Value::Array(items) => Value::Array(items.into_iter().map(sort_value).collect()),
        Value::Object(map) => {
            let sorted = map
                .into_iter()
                .map(|(key, value)| (key, sort_value(value)))
                .collect::<BTreeMap<_, _>>();
            Value::Object(sorted.into_iter().collect())
        }
        other => other,
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RepositoryExclusions {
    component_names: BTreeSet<String>,
    exact_paths: BTreeSet<PathBuf>,
    subtree_paths: BTreeSet<PathBuf>,
}

impl RepositoryExclusions {
    pub fn from_components(component_names: BTreeSet<String>) -> Self {
        Self {
            component_names,
            ..Self::default()
        }
    }

    pub fn excludes(&self, relative: &Path) -> bool {
        if path_has_ignored_component(relative, &self.component_names)
            || self.exact_paths.contains(relative)
        {
            return true;
        }
        let mut ancestor = Some(relative);
        while let Some(path) = ancestor {
            if self.subtree_paths.contains(path) {
                return true;
            }
            ancestor = path.parent();
        }
        false
    }
}

pub fn repo_owned_git_exclusions(
    repo: &Path,
    component_names: BTreeSet<String>,
) -> Result<RepositoryExclusions> {
    let output = Command::new("git")
        .env("GIT_OPTIONAL_LOCKS", "0")
        .arg("-C")
        .arg(repo)
        .args(["-c", "core.fsmonitor=false"])
        .args([
            "ls-files",
            "--others",
            "--ignored",
            "--exclude-per-directory=.gitignore",
            "--directory",
            "-z",
            "--",
            ".",
        ])
        .output()?;
    if !output.status.success() {
        bail!("cannot establish repository-owned Git ignore rules");
    }
    let mut exclusions = RepositoryExclusions::from_components(component_names);
    for raw in output.stdout.split(|byte| *byte == 0) {
        if raw.is_empty() {
            continue;
        }
        let text = std::str::from_utf8(raw).context("ignored Git path is not valid UTF-8")?;
        let subtree = text.ends_with('/');
        let normalized = text.strip_suffix('/').unwrap_or(text);
        if normalized.is_empty() {
            bail!("ignored Git path is empty");
        }
        let path = Path::new(normalized);
        if path.is_absolute()
            || path
                .components()
                .any(|component| !matches!(component, Component::Normal(_)))
        {
            bail!("ignored Git path is not a canonical repository-relative path");
        }
        if subtree {
            exclusions.subtree_paths.insert(path.to_path_buf());
        } else {
            exclusions.exact_paths.insert(path.to_path_buf());
        }
    }
    Ok(exclusions)
}

pub fn snapshot_repository(
    repo: &Path,
    exclusions: &RepositoryExclusions,
) -> Result<(Vec<SourceInput>, String, u64)> {
    let start = Instant::now();
    let mut sources = Vec::new();
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
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            continue;
        }
        let normalized_path = relative
            .components()
            .map(|component| component.as_os_str().to_string_lossy())
            .collect::<Vec<_>>()
            .join("/");
        if normalized_path.is_empty() {
            continue;
        }
        let mut file = File::open(entry.path())?;
        let mut bytes = Vec::with_capacity(metadata.len() as usize);
        file.read_to_end(&mut bytes)?;
        sources.push(SourceInput {
            artifact_id: format!("source:{normalized_path}"),
            normalized_path,
            content_digest: hash_bytes(&bytes),
            size_bytes: metadata.len(),
            origin: "USER".to_owned(),
        });
    }
    sources.sort_by(|left, right| left.artifact_id.cmp(&right.artifact_id));
    let tree = canonical_hash(&serde_json::json!({
        "schema":"codeclew.repository-tree/0.1",
        "members":sources,
    }))?;
    Ok((sources, tree, start.elapsed().as_micros() as u64))
}

fn path_has_ignored_component(path: &Path, ignored_components: &BTreeSet<String>) -> bool {
    path.components().any(|component| match component {
        Component::Normal(value) => value
            .to_str()
            .is_some_and(|value| ignored_components.contains(value)),
        _ => false,
    })
}

pub fn git_revision(repo: &Path) -> Result<Option<String>> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["rev-parse", "HEAD"])
        .output()?;
    if !output.status.success() {
        return Ok(None);
    }
    Ok(Some(String::from_utf8(output.stdout)?.trim().to_owned()))
}

pub fn git_dirty(repo: &Path) -> Result<bool> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args([
            "status",
            "--porcelain=v1",
            "--untracked-files=all",
            "--",
            ".",
        ])
        .output()?;
    if !output.status.success() {
        return Ok(true);
    }
    Ok(!output.stdout.is_empty())
}

pub fn executable_digest() -> Result<String> {
    let path = std::env::current_exe()?.canonicalize()?;
    let metadata = std::fs::symlink_metadata(&path)?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        bail!("adapter executable must be a regular non-symlink file");
    }
    let mut file = File::open(path)?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    Ok(hash_bytes(&bytes))
}

#[derive(Debug)]
enum QueryBoundaryScope {
    Irrelevant,
    Global,
    Exact {
        owner_keys: BTreeSet<String>,
        target_keys: BTreeSet<String>,
    },
}

fn string_key_set(value: Option<&Value>) -> Option<BTreeSet<String>> {
    let values = value?.as_array()?;
    let mut result = BTreeSet::new();
    for value in values {
        result.insert(value.as_str()?.to_owned());
    }
    Some(result)
}

fn query_boundary_scope(boundary: &Value, query_specification: &Value) -> QueryBoundaryScope {
    let Some(query_operations) = string_key_set(query_specification.get("operations")) else {
        return QueryBoundaryScope::Global;
    };
    if query_operations.is_empty()
        || query_specification.get("direction").and_then(Value::as_str)
            != Some(IMPACT_QUERY_DIRECTION)
    {
        return QueryBoundaryScope::Global;
    }
    let Some(applicability) = boundary.get("applicability").and_then(Value::as_object) else {
        // Legacy and malformed boundaries are deliberately fail-closed.
        return QueryBoundaryScope::Global;
    };
    let Some(owner_keys) = string_key_set(applicability.get("ownerQueryKeys")) else {
        return QueryBoundaryScope::Global;
    };
    let Some(target_keys) = string_key_set(applicability.get("targetQueryKeys")) else {
        return QueryBoundaryScope::Global;
    };
    let Some(boundary_operations) = string_key_set(applicability.get("operations")) else {
        return QueryBoundaryScope::Global;
    };
    if applicability.get("schema").and_then(Value::as_str)
        != Some("codeclew.boundary-applicability/0.1")
        || !matches!(
            applicability.get("direction").and_then(Value::as_str),
            Some("INCOMING" | "OUTGOING" | "BOTH")
        )
        || !matches!(
            applicability.get("locality").and_then(Value::as_str),
            Some("GLOBAL" | "EXACT")
        )
    {
        return QueryBoundaryScope::Global;
    }
    match applicability.get("effect").and_then(Value::as_str) {
        Some(BOUNDARY_EFFECT_ATTRIBUTE | BOUNDARY_EFFECT_BUILD_FIDELITY) => {
            return QueryBoundaryScope::Irrelevant;
        }
        Some(BOUNDARY_EFFECT_OUT_OF_SCOPE) => {
            return if applicability.get("entityScope").and_then(Value::as_str)
                == query_specification
                    .get("entityScope")
                    .and_then(Value::as_str)
            {
                QueryBoundaryScope::Irrelevant
            } else {
                // An unbound exclusion is not evidence that the entity is
                // outside the query. Keep legacy/malformed rows fail-closed.
                QueryBoundaryScope::Global
            };
        }
        Some(BOUNDARY_EFFECT_TOPOLOGY) => {}
        _ => return QueryBoundaryScope::Global,
    }
    // An empty provider operation set is deliberately fail-closed: it means
    // the provider could not bind the gap to a narrower relation family.
    if !boundary_operations.is_empty() && boundary_operations.is_disjoint(&query_operations) {
        return QueryBoundaryScope::Irrelevant;
    }
    match applicability.get("direction").and_then(Value::as_str) {
        Some("OUTGOING") => return QueryBoundaryScope::Irrelevant,
        Some("INCOMING" | "BOTH") => {}
        _ => return QueryBoundaryScope::Global,
    }
    match applicability.get("locality").and_then(Value::as_str) {
        Some("GLOBAL") => QueryBoundaryScope::Global,
        Some("EXACT") => {
            if target_keys.is_empty() {
                QueryBoundaryScope::Global
            } else {
                QueryBoundaryScope::Exact {
                    owner_keys,
                    target_keys,
                }
            }
        }
        _ => QueryBoundaryScope::Global,
    }
}

/// The agent-facing impact answer is a bounded reverse call graph, not an
/// arbitrary union of every semantic relation emitted by a provider.
pub fn impact_fact_is_in_scope(fact: &Value) -> bool {
    fact.get("truth").and_then(Value::as_str) == Some("TRUE")
        && fact.get("grade").and_then(Value::as_str) == Some("COMPILER_RESOLVED")
        && fact
            .get("relation")
            .and_then(Value::as_str)
            .is_some_and(|relation| IMPACT_QUERY_OPERATIONS.contains(&relation))
}

pub fn query_keys_for_entity(entity: &Value) -> Result<BTreeSet<String>> {
    let mut identities = BTreeSet::new();
    for pointer in [
        "/opaqueId",
        "/displayName",
        "/languagePayload/symbolIdentity",
        "/languagePayload/compilerCallableId",
        "/languagePayload/compilerClassId",
        "/languagePayload/ownerIdentity",
    ] {
        if let Some(identity) = entity.pointer(pointer).and_then(Value::as_str) {
            identities.insert(identity.to_owned());
        }
    }
    identities
        .iter()
        .map(|identity| query_endpoint_key(identity))
        .collect()
}

/// Applies one boundary to an incoming impact frontier. A `true` result means
/// the boundary can change this query's topology; exact boundaries also add
/// their unresolved owners so callers can continue the closure fixpoint.
pub fn apply_query_boundary_to_frontier(
    boundary: &Value,
    frontier_keys: &mut BTreeSet<String>,
) -> bool {
    match query_boundary_scope(boundary, &impact_query_specification()) {
        QueryBoundaryScope::Irrelevant => false,
        QueryBoundaryScope::Global => true,
        QueryBoundaryScope::Exact {
            owner_keys,
            target_keys,
        } => {
            if target_keys.is_disjoint(frontier_keys) {
                return false;
            }
            frontier_keys.extend(owner_keys);
            true
        }
    }
}

fn assess_query_boundaries(
    entities: &[Value],
    affected: &[Value],
    project_boundaries: &[Value],
    max_depth: usize,
) -> Result<(Vec<Value>, Value)> {
    let query_specification = impact_query_specification();
    let entity_index = entities
        .iter()
        .filter_map(|entity| {
            entity
                .get("opaqueId")
                .and_then(Value::as_str)
                .map(|id| (id, entity))
        })
        .collect::<BTreeMap<_, _>>();
    let mut frontier_keys = BTreeSet::new();
    for row in affected {
        // Incoming callers of an entity already at the traversal depth limit
        // would be one level outside the concrete bounded answer. Missing or
        // malformed internal depth metadata remains fail-closed.
        if row
            .get("depth")
            .and_then(Value::as_u64)
            .is_some_and(|depth| depth >= max_depth as u64)
        {
            continue;
        }
        let Some(entity_id) = row.get("entityId").and_then(Value::as_str) else {
            continue;
        };
        frontier_keys.insert(query_endpoint_key(entity_id)?);
        if let Some(entity) = entity_index.get(entity_id) {
            frontier_keys.extend(query_keys_for_entity(entity)?);
        }
    }

    let scopes = project_boundaries
        .iter()
        .map(|boundary| query_boundary_scope(boundary, &query_specification))
        .collect::<Vec<_>>();
    let mut relevant = vec![false; project_boundaries.len()];
    for (index, scope) in scopes.iter().enumerate() {
        if matches!(scope, QueryBoundaryScope::Global) {
            relevant[index] = true;
        }
    }
    loop {
        let mut changed = false;
        for (index, scope) in scopes.iter().enumerate() {
            if relevant[index] {
                continue;
            }
            let QueryBoundaryScope::Exact {
                owner_keys,
                target_keys,
            } = scope
            else {
                continue;
            };
            if target_keys.is_disjoint(&frontier_keys) {
                continue;
            }
            relevant[index] = true;
            let before = frontier_keys.len();
            frontier_keys.extend(owner_keys.iter().cloned());
            changed |= frontier_keys.len() != before;
        }
        if !changed {
            break;
        }
    }

    let query_boundaries = project_boundaries
        .iter()
        .zip(&relevant)
        .filter(|(_, relevant)| **relevant)
        .map(|(boundary, _)| boundary.clone())
        .collect::<Vec<_>>();
    let legacy_global_count = project_boundaries
        .iter()
        .filter(|boundary| boundary.get("applicability").is_none())
        .count();
    let assessment = serde_json::json!({
        "schema":"codeclew.query-boundary-assessment/0.1",
        "policy":"EXPLICIT_APPLICABILITY_FAIL_CLOSED",
        "querySpecificationDigest":canonical_hash(&query_specification)?,
        "projectBoundaryCount":project_boundaries.len(),
        "projectBoundarySetDigest":canonical_hash(&project_boundaries)?,
        "queryRelevantProjectBoundaryCount":query_boundaries.len(),
        "queryLocalBoundaryCount":0,
        "queryRelevantBoundaryCount":query_boundaries.len(),
        "queryRelevantBoundarySetDigest":canonical_hash(&query_boundaries)?,
        "legacyGlobalBoundaryCount":legacy_global_count,
    });
    Ok((query_boundaries, assessment))
}

fn update_query_boundary_assessment(
    assessment: &mut Value,
    query_boundaries: &[Value],
    query_local_count: usize,
) -> Result<()> {
    assessment["queryLocalBoundaryCount"] = Value::from(query_local_count as u64);
    assessment["queryRelevantBoundaryCount"] = Value::from(query_boundaries.len() as u64);
    assessment["queryRelevantBoundarySetDigest"] =
        Value::String(canonical_hash(&query_boundaries)?);
    Ok(())
}

fn closure_completeness_obligation(
    query_boundaries: &[Value],
    assessment: &Value,
    paths: &[Value],
) -> Result<Value> {
    let mut evidence_fact_ids = paths
        .iter()
        .filter_map(|path| path.get("factId").and_then(Value::as_str))
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();
    evidence_fact_ids.sort();
    evidence_fact_ids.dedup();
    let complete = query_boundaries.is_empty();
    let mut obligation = serde_json::json!({
        "id":"impact-closure-completeness",
        "kind":"codeclew.obligation/impact-closure-completeness/1",
        "mandatory":true,
        "status":if complete {"SATISFIED"} else {"UNKNOWN"},
        "evidenceFactIds":evidence_fact_ids,
        "providerPayload":{
            "boundaryAssessmentDigest":canonical_hash(assessment)?,
            "querySpecificationDigest":assessment.get("querySpecificationDigest"),
            "queryRelevantBoundaryCount":query_boundaries.len(),
            "queryRelevantBoundarySetDigest":canonical_hash(&query_boundaries)?,
        },
    });
    if !complete {
        obligation["reason"] = Value::String("QUERY_TOPOLOGY_BOUNDARY_REMAINS".to_owned());
    }
    Ok(obligation)
}

pub fn bounded_reverse_impact(
    seed: Option<&str>,
    entities: &[Value],
    facts: &[Value],
    boundaries: &[Value],
    max_depth: usize,
    max_entities: usize,
) -> Result<Value> {
    let start = Instant::now();
    let Some(seed) = seed else {
        let boundary = serde_json::json!({
            "boundaryId": canonical_hash(&serde_json::json!({
                "kind":"no-seed-entity",
                "closure":IMPACT_CLOSURE_SPEC,
            }))?,
            "kindUri":"codeclew.boundary/no-seed-entity/1",
            "consequence":"ENUMERATION_INCOMPLETE",
            "origin":Value::Null,
            "details":{"reason":"NO_SEED_ENTITY"},
            "applicability":{
                "effect":BOUNDARY_EFFECT_TOPOLOGY,
                "direction":"BOTH",
                "locality":"GLOBAL",
                "ownerQueryKeys":[],
                "targetQueryKeys":[],
            },
        });
        let (mut impact_boundaries, mut assessment) =
            assess_query_boundaries(entities, &[], boundaries, max_depth)?;
        impact_boundaries.push(boundary);
        update_query_boundary_assessment(&mut assessment, &impact_boundaries, 1)?;
        let closure = closure_completeness_obligation(&impact_boundaries, &assessment, &[])?;
        return Ok(serde_json::json!({
            "schema":"codeclew.impact-result/0.1",
            "status":"UNKNOWN",
            "reason":"NO_SEED_ENTITY",
            "closureSpecification":IMPACT_CLOSURE_SPEC,
            "querySpecification":impact_query_specification(),
            "affected":[],
            "paths":[],
            "mandatoryObligations":[{
                "id":"resolve-seed",
                "kind":"codeclew.obligation/resolve-entity/1",
                "mandatory":true,
                "status":"UNKNOWN",
                "reason":"NO_SEED_ENTITY",
            }, closure],
            "boundaries":impact_boundaries,
            "boundaryAssessment":assessment,
            "queryMicros":start.elapsed().as_micros() as u64,
        }));
    };
    if !entities
        .iter()
        .any(|entity| entity.get("opaqueId").and_then(Value::as_str) == Some(seed))
    {
        let (impact_boundaries, assessment) =
            assess_query_boundaries(entities, &[], boundaries, max_depth)?;
        let closure = closure_completeness_obligation(&impact_boundaries, &assessment, &[])?;
        return Ok(serde_json::json!({
            "schema":"codeclew.impact-result/0.1",
            "status":"UNKNOWN",
            "reason":"UNRESOLVED_SEED_ENTITY",
            "closureSpecification":IMPACT_CLOSURE_SPEC,
            "querySpecification":impact_query_specification(),
            "seedEntity":seed,
            "affected":[],
            "paths":[],
            "mandatoryObligations":[{
                "id":"resolve-seed",
                "kind":"codeclew.obligation/resolve-entity/1",
                "mandatory":true,
                "status":"UNKNOWN"
            }, closure],
            "boundaries":impact_boundaries,
            "boundaryAssessment":assessment,
            "queryMicros":start.elapsed().as_micros() as u64,
        }));
    }

    let mut queue = VecDeque::from([(seed.to_owned(), 0usize)]);
    let mut visited = BTreeSet::from([seed.to_owned()]);
    let mut affected = vec![serde_json::json!({
        "entityId":seed,
        "impactClass":"DEFINITE",
        "depth":0,
    })];
    let mut paths = Vec::new();
    while let Some((target, depth)) = queue.pop_front() {
        if depth >= max_depth || affected.len() >= max_entities {
            continue;
        }
        for fact in facts {
            if !impact_fact_is_in_scope(fact)
                || fact.get("target").and_then(Value::as_str) != Some(target.as_str())
            {
                continue;
            }
            let Some(owner) = fact.get("owner").and_then(Value::as_str) else {
                continue;
            };
            paths.push(serde_json::json!({
                "from":target,
                "to":owner,
                "factId":fact.get("factId").cloned().unwrap_or(Value::Null),
                "relation":fact.get("relation").cloned().unwrap_or(Value::Null),
            }));
            if visited.insert(owner.to_owned()) {
                affected.push(serde_json::json!({
                    "entityId":owner,
                    "impactClass":"POSSIBLE",
                    "depth":depth + 1,
                }));
                queue.push_back((owner.to_owned(), depth + 1));
            }
            if affected.len() >= max_entities {
                break;
            }
        }
    }
    let budget_boundary = affected.len() >= max_entities;
    let (mut impact_boundaries, mut assessment) =
        assess_query_boundaries(entities, &affected, boundaries, max_depth)?;
    let mut query_local_boundary_count = 0usize;
    if budget_boundary {
        let boundary = serde_json::json!({
            "boundaryId":canonical_hash(&serde_json::json!({
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
        });
        impact_boundaries.push(boundary);
        query_local_boundary_count += 1;
    }
    update_query_boundary_assessment(
        &mut assessment,
        &impact_boundaries,
        query_local_boundary_count,
    )?;
    let status = if impact_boundaries.is_empty() {
        "COMPLETE_IN_SCOPE"
    } else {
        "PARTIAL_BOUNDARY"
    };
    let closure = closure_completeness_obligation(&impact_boundaries, &assessment, &paths)?;
    Ok(serde_json::json!({
        "schema":"codeclew.impact-result/0.1",
        "status":status,
        "closureSpecification":IMPACT_CLOSURE_SPEC,
        "querySpecification":impact_query_specification(),
        "seedEntity":seed,
        "maxDepth":max_depth,
        "maxEntities":max_entities,
        "affected":affected,
        "paths":paths,
        "mandatoryObligations":[closure],
        "boundaries":impact_boundaries,
        "boundaryAssessment":assessment,
        "queryMicros":start.elapsed().as_micros() as u64,
    }))
}

pub fn required_string<'a>(value: &'a Value, pointer: &str) -> Result<&'a str> {
    value
        .pointer(pointer)
        .and_then(Value::as_str)
        .with_context(|| format!("adapter input is missing string at {pointer}"))
}

pub fn normalize_repo(path: &Path) -> Result<PathBuf> {
    let canonical = path.canonicalize()?;
    let metadata = std::fs::symlink_metadata(&canonical)?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        bail!("repository must be a regular non-symlink directory");
    }
    Ok(canonical)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn impact_never_calls_partial_graph_complete() {
        let entities = vec![serde_json::json!({"opaqueId":"target"})];
        let impact = bounded_reverse_impact(
            Some("target"),
            &entities,
            &[],
            &[serde_json::json!({
                "kindUri":"codeclew.boundary/dynamic-dispatch/1",
                "consequence":"ENUMERATION_INCOMPLETE"
            })],
            2,
            20,
        )
        .unwrap();
        assert_eq!(impact["status"], "PARTIAL_BOUNDARY");
        assert_eq!(impact["mandatoryObligations"][0]["status"], "UNKNOWN");
        assert_eq!(impact["mandatoryObligations"].as_array().unwrap().len(), 1);
        assert_eq!(impact["boundaryAssessment"]["legacyGlobalBoundaryCount"], 1);
    }

    #[test]
    fn explicit_attribute_only_boundary_is_not_a_query_unknown() {
        let entities = vec![serde_json::json!({"opaqueId":"target"})];
        let boundary = serde_json::json!({
            "boundaryId":hash_bytes(b"attribute"),
            "kindUri":"codeclew.boundary/test/attribute/1",
            "consequence":"ENUMERATION_INCOMPLETE",
            "applicability":{
                "schema":"codeclew.boundary-applicability/0.1",
                "effect":BOUNDARY_EFFECT_ATTRIBUTE,
                "operations":[],
                "direction":"INCOMING",
                "locality":"GLOBAL",
                "ownerQueryKeys":[],
                "targetQueryKeys":[],
            },
        });
        let impact =
            bounded_reverse_impact(Some("target"), &entities, &[], &[boundary], 2, 20).unwrap();
        assert_eq!(impact["status"], "COMPLETE_IN_SCOPE");
        assert!(impact["boundaries"].as_array().unwrap().is_empty());
        assert_eq!(impact["mandatoryObligations"][0]["status"], "SATISFIED");
        assert_eq!(impact["boundaryAssessment"]["projectBoundaryCount"], 1);
        assert_eq!(
            impact["boundaryAssessment"]["queryRelevantBoundaryCount"],
            0
        );
    }

    #[test]
    fn call_graph_query_ignores_bound_other_operations_but_not_unbound_gaps() {
        let entities = vec![serde_json::json!({"opaqueId":"target"})];
        let boundary = |operations: Value| {
            serde_json::json!({
                "boundaryId":canonical_hash(&operations).unwrap(),
                "kindUri":"codeclew.boundary/test/operation/1",
                "consequence":"ENUMERATION_INCOMPLETE",
                "applicability":{
                    "schema":"codeclew.boundary-applicability/0.1",
                    "effect":BOUNDARY_EFFECT_TOPOLOGY,
                    "operations":operations,
                    "direction":"INCOMING",
                    "locality":"GLOBAL",
                    "ownerQueryKeys":[],
                    "targetQueryKeys":[],
                },
            })
        };
        let impact = bounded_reverse_impact(
            Some("target"),
            &entities,
            &[],
            &[
                boundary(serde_json::json!([
                    "codeclew.relation/returns_value_from/1"
                ])),
                boundary(serde_json::json!([])),
            ],
            2,
            20,
        )
        .unwrap();
        assert_eq!(
            impact["querySpecification"]["operations"],
            serde_json::json!([
                "codeclew.relation/calls/1",
                "codeclew.relation/constructs/1",
            ])
        );
        assert_eq!(impact["boundaries"].as_array().unwrap().len(), 1);
        assert_eq!(impact["status"], "PARTIAL_BOUNDARY");
    }

    #[test]
    fn reverse_call_graph_traverses_only_exact_compiler_resolved_operations() {
        let entities = vec![
            serde_json::json!({"opaqueId":"target"}),
            serde_json::json!({"opaqueId":"caller"}),
            serde_json::json!({"opaqueId":"reader"}),
            serde_json::json!({"opaqueId":"implementor"}),
        ];
        let fact = |id: &str, owner: &str, relation: &str, grade: &str| {
            serde_json::json!({
                "factId":id,
                "truth":"TRUE",
                "grade":grade,
                "owner":owner,
                "target":"target",
                "relation":relation,
            })
        };
        let impact = bounded_reverse_impact(
            Some("target"),
            &entities,
            &[
                fact(
                    "call",
                    "caller",
                    "codeclew.relation/calls/1",
                    "COMPILER_RESOLVED",
                ),
                fact(
                    "reference",
                    "reader",
                    "codeclew.relation/references/1",
                    "COMPILER_RESOLVED",
                ),
                fact(
                    "weak",
                    "reader",
                    "codeclew.relation/calls/1",
                    "STATICALLY_APPROXIMATED",
                ),
                fact(
                    "override",
                    "implementor",
                    "codeclew.relation/overrides/1",
                    "COMPILER_RESOLVED",
                ),
            ],
            &[],
            2,
            20,
        )
        .unwrap();
        assert_eq!(impact["paths"].as_array().unwrap().len(), 1);
        assert_eq!(impact["paths"][0]["factId"], "call");
        assert_eq!(impact["affected"].as_array().unwrap().len(), 2);
        assert!(!impact_fact_is_in_scope(&fact(
            "override",
            "implementor",
            "codeclew.relation/overrides/1",
            "COMPILER_RESOLVED",
        )));
    }

    #[test]
    fn out_of_scope_boundary_requires_the_declared_entity_scope() {
        let entities = vec![serde_json::json!({"opaqueId":"target"})];
        let boundary = |entity_scope: Option<&str>| {
            let mut applicability = serde_json::json!({
                "schema":"codeclew.boundary-applicability/0.1",
                "effect":BOUNDARY_EFFECT_OUT_OF_SCOPE,
                "operations":[],
                "direction":"INCOMING",
                "locality":"GLOBAL",
                "ownerQueryKeys":[],
                "targetQueryKeys":[],
            });
            if let Some(entity_scope) = entity_scope {
                applicability["entityScope"] = Value::String(entity_scope.to_owned());
            }
            serde_json::json!({
                "boundaryId":hash_bytes(entity_scope.unwrap_or("unbound").as_bytes()),
                "kindUri":"codeclew.boundary/test/generated-or-no-source/1",
                "consequence":"ENUMERATION_INCOMPLETE",
                "applicability":applicability,
            })
        };
        let scoped = bounded_reverse_impact(
            Some("target"),
            &entities,
            &[],
            &[boundary(Some(IMPACT_QUERY_ENTITY_SCOPE))],
            2,
            20,
        )
        .unwrap();
        assert_eq!(scoped["status"], "COMPLETE_IN_SCOPE");
        assert!(scoped["boundaries"].as_array().unwrap().is_empty());
        assert_eq!(
            scoped["querySpecification"]["entityScope"],
            IMPACT_QUERY_ENTITY_SCOPE
        );

        let unbound =
            bounded_reverse_impact(Some("target"), &entities, &[], &[boundary(None)], 2, 20)
                .unwrap();
        assert_eq!(unbound["status"], "PARTIAL_BOUNDARY");
        assert_eq!(unbound["boundaries"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn exact_topology_boundary_is_relevant_only_when_target_intersects_frontier() {
        let entities = vec![serde_json::json!({
            "opaqueId":"target",
            "languagePayload":{"compilerCallableId":"p.Target.call"},
        })];
        let make_boundary = |target: &str| {
            serde_json::json!({
                "boundaryId":hash_bytes(target.as_bytes()),
                "kindUri":"codeclew.boundary/test/exact/1",
                "consequence":"ENUMERATION_INCOMPLETE",
                "applicability":{
                    "schema":"codeclew.boundary-applicability/0.1",
                    "effect":BOUNDARY_EFFECT_TOPOLOGY,
                    "operations":["codeclew.relation/calls/1"],
                    "direction":"INCOMING",
                    "locality":"EXACT",
                    "ownerQueryKeys":[],
                    "targetQueryKeys":[query_endpoint_key(target).unwrap()],
                },
            })
        };
        let outside = bounded_reverse_impact(
            Some("target"),
            &entities,
            &[],
            &[make_boundary("p.Other.call")],
            2,
            20,
        )
        .unwrap();
        assert_eq!(outside["status"], "COMPLETE_IN_SCOPE");
        assert!(outside["boundaries"].as_array().unwrap().is_empty());

        let inside = bounded_reverse_impact(
            Some("target"),
            &entities,
            &[],
            &[make_boundary("p.Target.call")],
            2,
            20,
        )
        .unwrap();
        assert_eq!(inside["status"], "PARTIAL_BOUNDARY");
        assert_eq!(inside["boundaries"].as_array().unwrap().len(), 1);
        assert_eq!(inside["mandatoryObligations"][0]["status"], "UNKNOWN");
    }

    #[test]
    fn exact_boundary_at_terminal_depth_cannot_change_the_bounded_answer() {
        let entities = vec![
            serde_json::json!({"opaqueId":"seed"}),
            serde_json::json!({"opaqueId":"caller"}),
            serde_json::json!({"opaqueId":"terminal"}),
        ];
        let facts = vec![
            serde_json::json!({
                "factId":"seed-caller",
                "truth":"TRUE",
                "grade":"COMPILER_RESOLVED",
                "owner":"caller",
                "target":"seed",
                "relation":"codeclew.relation/calls/1",
            }),
            serde_json::json!({
                "factId":"caller-terminal",
                "truth":"TRUE",
                "grade":"COMPILER_RESOLVED",
                "owner":"terminal",
                "target":"caller",
                "relation":"codeclew.relation/calls/1",
            }),
        ];
        let boundary = |target: &str| {
            serde_json::json!({
                "boundaryId":hash_bytes(target.as_bytes()),
                "kindUri":"codeclew.boundary/test/exact-depth/1",
                "consequence":"ENUMERATION_INCOMPLETE",
                "applicability":{
                    "schema":"codeclew.boundary-applicability/0.1",
                    "effect":BOUNDARY_EFFECT_TOPOLOGY,
                    "operations":["codeclew.relation/calls/1"],
                    "direction":"INCOMING",
                    "locality":"EXACT",
                    "ownerQueryKeys":[query_endpoint_key("unknown-caller").unwrap()],
                    "targetQueryKeys":[query_endpoint_key(target).unwrap()],
                },
            })
        };

        let terminal = bounded_reverse_impact(
            Some("seed"),
            &entities,
            &facts,
            &[boundary("terminal")],
            2,
            20,
        )
        .unwrap();
        assert_eq!(terminal["affected"][2]["depth"], 2);
        assert_eq!(terminal["status"], "COMPLETE_IN_SCOPE");
        assert!(terminal["boundaries"].as_array().unwrap().is_empty());

        let traversable = bounded_reverse_impact(
            Some("seed"),
            &entities,
            &facts,
            &[boundary("caller")],
            2,
            20,
        )
        .unwrap();
        assert_eq!(traversable["affected"][1]["depth"], 1);
        assert_eq!(traversable["status"], "PARTIAL_BOUNDARY");
        assert_eq!(traversable["boundaries"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn unknown_applicability_effect_fails_closed_as_global_topology() {
        let entities = vec![serde_json::json!({"opaqueId":"target"})];
        let boundary = serde_json::json!({
            "boundaryId":hash_bytes(b"unknown-effect"),
            "kindUri":"codeclew.boundary/test/unknown/1",
            "consequence":"ENUMERATION_INCOMPLETE",
            "applicability":{
                "schema":"codeclew.boundary-applicability/0.1",
                "effect":"MAYBE_ATTRIBUTE",
                "operations":[],
                "direction":"OUTGOING",
                "locality":"EXACT",
                "ownerQueryKeys":[],
                "targetQueryKeys":[],
            },
        });
        let impact =
            bounded_reverse_impact(Some("target"), &entities, &[], &[boundary], 2, 20).unwrap();
        assert_eq!(impact["status"], "PARTIAL_BOUNDARY");
        assert_eq!(impact["boundaries"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn repository_tree_changes_with_dirty_content() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("source.kt"), "fun before() = 1\n").unwrap();
        let exclusions = RepositoryExclusions::default();
        let (_, before, _) = snapshot_repository(dir.path(), &exclusions).unwrap();
        std::fs::write(dir.path().join("source.kt"), "fun after() = 2\n").unwrap();
        let (_, after, _) = snapshot_repository(dir.path(), &exclusions).unwrap();
        assert_ne!(before, after);
    }

    #[test]
    fn repository_snapshot_prunes_ignored_subtrees() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("source.kt"), "fun source() = 1\n").unwrap();
        let ignored = dir.path().join("node_modules");
        std::fs::create_dir(&ignored).unwrap();
        std::fs::write(ignored.join("large-runtime-file"), vec![b'x'; 1024 * 1024]).unwrap();

        let exclusions =
            RepositoryExclusions::from_components(BTreeSet::from(["node_modules".to_owned()]));
        let (sources, _, _) = snapshot_repository(dir.path(), &exclusions).unwrap();

        assert_eq!(sources.len(), 1);
        assert_eq!(sources[0].normalized_path, "source.kt");
    }

    #[test]
    fn output_seal_binds_exact_canonical_size_and_content() {
        let mut output = AdapterOutput {
            schema: ADAPTER_OUTPUT_SCHEMA.to_owned(),
            adapter: AdapterIdentity {
                adapter_id: "adapter".to_owned(),
                version: "1".to_owned(),
                binary_digest: hash_bytes(b"adapter"),
                language_id: "test".to_owned(),
            },
            snapshot_input: SnapshotInput {
                repository_tree_digest: hash_bytes(b"tree"),
                vcs_revision: None,
                dirty: true,
                sources: vec![],
                build_system_uri: "build:test".to_owned(),
                build_model_digest: hash_bytes(b"model"),
                build_configuration_digest: hash_bytes(b"config"),
                dependency_graph_digest: hash_bytes(b"deps"),
                toolchain: serde_json::json!({}),
                targets: vec![],
                relevant_environment: vec![],
                generated_sources_manifest_digest: hash_bytes(b"generated"),
            },
            capability_descriptors: vec![],
            entities: vec![],
            occurrences: vec![],
            facts: vec![],
            boundaries: vec![],
            compiler_receipt: serde_json::json!({}),
            impact: serde_json::json!({}),
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
        };
        output.seal().unwrap();
        output.verify_seal().unwrap();
        output.adapter.version.push('x');
        assert!(output.verify_seal().is_err());
    }
}
