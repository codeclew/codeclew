//! K1-only terminal-attempt and semantic-cache protocol for the Kotlin
//! adapter. This module is intentionally adapter-owned: it does not widen the
//! frozen K0.1 evidence protocol or the shared evidence runtime.

use anyhow::{Context, Result, bail};
use evidence_adapters::{
    AdapterOutput, BOUNDARY_EFFECT_ATTRIBUTE, BOUNDARY_EFFECT_BUILD_FIDELITY,
    BOUNDARY_EFFECT_OUT_OF_SCOPE, BOUNDARY_EFFECT_TOPOLOGY, CoreBindingSummary,
    IMPACT_QUERY_ENTITY_SCOPE, canonical_bytes, canonical_hash, hash_bytes, validate_core_binding,
};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{File, OpenOptions};
use std::io::{ErrorKind, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

pub const ATTEMPT_SCHEMA: &str = "codeclew.kotlin-real-repository-attempt/0.1";
pub const PREPARED_REFUSAL_SCHEMA: &str = "codeclew.kotlin-k1-dependency-preparation-refusal/0.11";
pub const K1_SERIES_ID: &str = "KOTLIN_REAL_REPOSITORY_K1_12_2026_08_13";
const CACHE_SCHEMA: &str = "codeclew.kotlin-semantic-cache-object/0.1";
const CACHE_INPUT_SCHEMA: &str = "codeclew.kotlin-semantic-cache-input/0.1";
pub const AGENT_GRAPH_NORMALIZATION_CONTRACT: &str =
    "codeclew.kotlin-agent-graph-normalization/0.5";
const AGENT_CACHE_SELECTOR_SCHEMA: &str = "codeclew.kotlin-agent-cache-selector/0.1";
const AGENT_CACHE_CATALOG_SCHEMA: &str = "codeclew.kotlin-agent-cache-catalog/0.1";
const AGENT_GRAPH_MANIFEST_SCHEMA: &str = "codeclew.kotlin-agent-graph-manifest/0.1";
const AGENT_GRAPH_SHARD_SCHEMA: &str = "codeclew.kotlin-agent-graph-shard/0.1";
const AGENT_GRAPH_BOUNDARY_SET_SCHEMA: &str = "codeclew.kotlin-agent-boundary-set/0.1";
const AGENT_GRAPH_BUCKET_HEX_DIGITS: usize = 2;
static TEMPORARY_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PreparedRefusalCost {
    pub wall_micros: u64,
    pub stdout_bytes: u64,
    pub stderr_bytes: u64,
    pub exit_code: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PreparedRefusal {
    pub schema: String,
    pub series_id: String,
    pub cohort: String,
    pub entry: String,
    pub commit: String,
    pub git_tree: String,
    pub selected_compilation: String,
    pub build_dsl: String,
    pub failure_stage: String,
    pub reason_code: String,
    pub safe_detail_digest: String,
    pub cost: PreparedRefusalCost,
    pub sandbox_profile_sha256: String,
    pub source_tree_sha256: String,
    pub candidate_tools_sha256: String,
    pub build_input_digest: String,
    pub preparation_receipt_digest: String,
    pub object_digest: String,
}

impl PreparedRefusal {
    pub fn verify(&self) -> Result<()> {
        if self.schema != PREPARED_REFUSAL_SCHEMA || self.series_id != K1_SERIES_ID {
            bail!("prepared refusal schema or series identity mismatch");
        }
        if !matches!(self.cohort.as_str(), "QUALIFICATION" | "BLIND_HOLDOUT") {
            bail!("prepared refusal cohort is not a K1 corpus cohort");
        }
        if self.entry.is_empty()
            || self.selected_compilation.is_empty()
            || !matches!(
                self.build_dsl.as_str(),
                "MAVEN" | "GRADLE_KOTLIN_DSL" | "GRADLE_GROOVY_DSL"
            )
            || self.failure_stage != "DEPENDENCY_PREPARATION"
            || !matches!(
                self.reason_code.as_str(),
                "DEPENDENCY_CLOSURE_UNAVAILABLE"
                    | "OFFLINE_MODEL_PROBE_FAILED"
                    | "UNSUPPORTED_BUILD_CONFIGURATION"
            )
            || self.cost.exit_code == 0
        {
            bail!("prepared refusal semantic fields are invalid");
        }
        require_git_object(&self.commit)?;
        require_git_object(&self.git_tree)?;
        for digest in [
            &self.safe_detail_digest,
            &self.sandbox_profile_sha256,
            &self.source_tree_sha256,
            &self.candidate_tools_sha256,
            &self.build_input_digest,
            &self.preparation_receipt_digest,
            &self.object_digest,
        ] {
            require_digest(digest)?;
        }
        let expected = self.object_digest.clone();
        let mut projection = self.clone();
        projection.object_digest.clear();
        if canonical_hash(&projection)? != expected {
            bail!("prepared refusal objectDigest self-seal mismatch");
        }
        Ok(())
    }
}

pub fn read_prepared_refusal(
    path: &Path,
    repository: &Path,
    expected_file_digest: &str,
) -> Result<PreparedRefusal> {
    require_digest(expected_file_digest)?;
    if !path.is_absolute() {
        bail!("prepared refusal path must be absolute");
    }
    let canonical_path = path
        .canonicalize()
        .context("prepared refusal must already exist")?;
    if canonical_path != path {
        bail!("prepared refusal path must be canonical and have no symlinked ancestor");
    }
    let repository = repository.canonicalize()?;
    if canonical_path.starts_with(&repository) || repository.starts_with(&canonical_path) {
        bail!("prepared refusal must be outside and must not contain the source checkout");
    }
    let metadata = std::fs::symlink_metadata(&canonical_path)?;
    if metadata.len() == 0 || metadata.len() > 1024 * 1024 {
        bail!("prepared refusal size must be in 1..=1048576 bytes");
    }
    let bytes = read_immutable_regular(&canonical_path)?;
    if hash_bytes(&bytes) != expected_file_digest {
        bail!("prepared refusal bytes differ from --prepared-refusal-sha256");
    }
    let refusal: PreparedRefusal =
        serde_json::from_slice(&bytes).context("prepared refusal is invalid JSON")?;
    let mut canonical = canonical_bytes(&refusal)?;
    canonical.push(b'\n');
    if bytes != canonical {
        bail!("prepared refusal is not canonical JSON plus newline");
    }
    refusal.verify()?;
    Ok(refusal)
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AttemptTelemetry {
    pub external_wall_micros: u64,
    pub maximum_resident_bytes: Value,
    pub source_hashing_micros: u64,
    pub build_discovery_micros: u64,
    pub dependency_preparation_micros: Value,
    pub dependency_verification_micros: Value,
    pub adapter_startup_micros: u64,
    pub cold_index_micros: u64,
    pub warm_index_micros: u64,
    pub provider_processing_micros: u64,
    pub serialization_micros: u64,
    pub store_write_micros: u64,
    pub store_read_micros: u64,
    pub query_projection_micros: u64,
    pub source_bytes_read: u64,
    pub cache_bytes_read: u64,
    pub cache_bytes_written: u64,
    pub emitted_bytes: u64,
    pub stored_fact_bytes: u64,
    pub fact_count: u64,
    pub boundary_count: u64,
    pub cache_requests: u64,
    pub cache_hits: u64,
    pub model_calls: u64,
}

impl AttemptTelemetry {
    pub fn new() -> Self {
        Self {
            maximum_resident_bytes: Value::String("UNKNOWN".to_owned()),
            dependency_preparation_micros: Value::String("NOT_IN_THIS_INVOCATION".to_owned()),
            dependency_verification_micros: Value::String("UNKNOWN".to_owned()),
            ..Self::default()
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct KotlinAttempt {
    pub schema: String,
    pub status: String,
    pub outcome_kind: String,
    pub failure_stage: Option<String>,
    pub reason_code: Option<String>,
    pub detail_digest: Option<String>,
    pub selected_inputs: Value,
    pub snapshot: Value,
    pub provenance: Value,
    pub boundaries: Vec<Value>,
    pub adapter_output_digest: Option<String>,
    pub evidence_core: Option<CoreBindingSummary>,
    pub cache: Value,
    pub cost: AttemptTelemetry,
    pub terminal_semantic_digest: String,
    pub attempt_digest: String,
}

impl KotlinAttempt {
    pub fn success(
        output: &AdapterOutput,
        core: CoreBindingSummary,
        selected_inputs: Value,
        provenance: Value,
        cache: Value,
        cost: AttemptTelemetry,
    ) -> Result<Self> {
        let snapshot = snapshot_from_output(output);
        let mut attempt = Self {
            schema: ATTEMPT_SCHEMA.to_owned(),
            status: "SUCCEEDED".to_owned(),
            outcome_kind: "ADAPTER_OUTPUT".to_owned(),
            failure_stage: None,
            reason_code: None,
            detail_digest: None,
            selected_inputs,
            snapshot,
            provenance,
            boundaries: output.boundaries.clone(),
            adapter_output_digest: Some(output.output_digest.clone()),
            evidence_core: Some(core),
            cache,
            cost,
            terminal_semantic_digest: semantic_output_digest(output)?,
            attempt_digest: String::new(),
        };
        attempt.seal()?;
        Ok(attempt)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn terminal(
        status: &str,
        stage: &str,
        reason_code: &str,
        detail: &str,
        selected_inputs: Value,
        snapshot: Value,
        provenance: Value,
        boundaries: Vec<Value>,
        cache: Value,
        cost: AttemptTelemetry,
    ) -> Result<Self> {
        let detail_digest = hash_bytes(safe_detail_identity(detail).as_bytes());
        Self::terminal_with_detail_digest(
            status,
            stage,
            reason_code,
            &detail_digest,
            selected_inputs,
            snapshot,
            provenance,
            boundaries,
            cache,
            cost,
        )
    }

    /// Constructs a typed terminal from a digest issued by a separately
    /// sealed, adapter-validated authority object. This is deliberately not a
    /// general diagnostic escape hatch: callers must first validate that the
    /// digest is bound into that object's self-seal.
    #[allow(clippy::too_many_arguments)]
    pub fn terminal_with_detail_digest(
        status: &str,
        stage: &str,
        reason_code: &str,
        detail_digest: &str,
        selected_inputs: Value,
        snapshot: Value,
        provenance: Value,
        boundaries: Vec<Value>,
        cache: Value,
        cost: AttemptTelemetry,
    ) -> Result<Self> {
        if !matches!(status, "PARTIAL" | "REFUSED" | "FAILED") {
            bail!("invalid terminal attempt status {status}");
        }
        require_digest(detail_digest)?;
        let terminal_semantic_digest = canonical_hash(&json!({
            "schema":ATTEMPT_SCHEMA,
            "status":status,
            "failureStage":stage,
            "reasonCode":reason_code,
            "detailDigest":detail_digest,
            "selectedInputs":selected_inputs,
            "snapshot":snapshot,
            "provenance":provenance,
            "boundaries":boundaries,
            "cache":cache,
        }))?;
        let mut attempt = Self {
            schema: ATTEMPT_SCHEMA.to_owned(),
            status: status.to_owned(),
            outcome_kind: "TYPED_TERMINAL".to_owned(),
            failure_stage: Some(stage.to_owned()),
            reason_code: Some(reason_code.to_owned()),
            detail_digest: Some(detail_digest.to_owned()),
            selected_inputs,
            snapshot,
            provenance,
            boundaries,
            adapter_output_digest: None,
            evidence_core: None,
            cache,
            cost,
            terminal_semantic_digest,
            attempt_digest: String::new(),
        };
        attempt.seal()?;
        Ok(attempt)
    }

    pub fn seal(&mut self) -> Result<()> {
        self.attempt_digest.clear();
        self.attempt_digest = canonical_hash(self)?;
        Ok(())
    }

    pub fn seal_for_stdout(&mut self) -> Result<()> {
        loop {
            self.seal()?;
            let emitted = canonical_bytes(self)?.len() as u64 + 1;
            if self.cost.emitted_bytes == emitted {
                return Ok(());
            }
            self.cost.emitted_bytes = emitted;
        }
    }

    pub fn verify(&self) -> Result<()> {
        if self.schema != ATTEMPT_SCHEMA {
            bail!("unsupported Kotlin attempt schema");
        }
        if self.cost.model_calls != 0 {
            bail!("Kotlin K1 attempts must report modelCalls=0");
        }
        if self.cost.cache_hits > self.cost.cache_requests {
            bail!("Kotlin K1 attempt cache hits exceed cache requests");
        }
        match self.status.as_str() {
            "SUCCEEDED"
                if self.outcome_kind == "ADAPTER_OUTPUT"
                    && self.failure_stage.is_none()
                    && self.reason_code.is_none()
                    && self.detail_digest.is_none()
                    && self.adapter_output_digest.is_some()
                    && self.evidence_core.is_some() =>
            {
                let semantic_output = self
                    .cache
                    .get("semanticOutputDigest")
                    .and_then(Value::as_str)
                    .context("successful attempt cache misses semanticOutputDigest")?;
                let semantic_facts = self
                    .cache
                    .get("semanticFactsDigest")
                    .and_then(Value::as_str)
                    .context("successful attempt cache misses semanticFactsDigest")?;
                let key = self
                    .cache
                    .get("keyDigest")
                    .and_then(Value::as_str)
                    .context("successful attempt cache misses keyDigest")?;
                require_digest(semantic_output)?;
                require_digest(semantic_facts)?;
                require_digest(key)?;
                if semantic_output != self.terminal_semantic_digest {
                    bail!("successful attempt semantic digest differs from cache authority");
                }
                let phase = self.selected_inputs.get("runPhase").and_then(Value::as_str);
                match (
                    self.cache.get("status").and_then(Value::as_str),
                    self.cache.get("hit").and_then(Value::as_bool),
                    phase,
                ) {
                    (Some("PUBLISHED_COLD"), Some(false), Some("COLD"))
                        if self.cost.cache_requests >= 1 && self.cost.cache_bytes_written > 0 => {}
                    (Some("VERIFIED_HIT"), Some(true), Some("WARM"))
                        if self.cost.cache_requests >= 1
                            && self.cost.cache_hits >= 1
                            && self.cost.cache_bytes_read > 0
                            && self.cost.cold_index_micros == 0 => {}
                    _ => bail!("successful attempt cache telemetry is not authoritative"),
                }
            }
            "PARTIAL" | "REFUSED" | "FAILED"
                if self.outcome_kind == "TYPED_TERMINAL"
                    && self
                        .failure_stage
                        .as_deref()
                        .is_some_and(|value| !value.is_empty())
                    && self
                        .reason_code
                        .as_deref()
                        .is_some_and(|value| !value.is_empty())
                    && self.detail_digest.is_some()
                    && self.adapter_output_digest.is_none()
                    && self.evidence_core.is_none() => {}
            _ => bail!("Kotlin attempt terminal fields are incoherent with status"),
        }
        require_digest(&self.terminal_semantic_digest)?;
        require_digest(&self.attempt_digest)?;
        if let Some(digest) = &self.detail_digest {
            require_digest(digest)?;
        }
        if let Some(digest) = &self.adapter_output_digest {
            require_digest(digest)?;
        }
        if self.status != "SUCCEEDED" {
            let expected_terminal_semantics = canonical_hash(&json!({
                "schema":ATTEMPT_SCHEMA,
                "status":self.status,
                "failureStage":self.failure_stage,
                "reasonCode":self.reason_code,
                "detailDigest":self.detail_digest,
                "selectedInputs":self.selected_inputs,
                "snapshot":self.snapshot,
                "provenance":self.provenance,
                "boundaries":self.boundaries,
                "cache":self.cache,
            }))?;
            if self.terminal_semantic_digest != expected_terminal_semantics {
                bail!("Kotlin terminal semantic digest mismatch");
            }
        }
        let expected = self.attempt_digest.clone();
        let mut projection = self.clone();
        projection.attempt_digest.clear();
        if canonical_hash(&projection)? != expected {
            bail!("Kotlin attempt digest mismatch");
        }
        Ok(())
    }
}

fn redact_absolute_paths(detail: &str) -> String {
    let mut result = String::with_capacity(detail.len());
    let characters = detail.chars().collect::<Vec<_>>();
    let mut index = 0;
    while index < characters.len() {
        let starts_path = characters[index] == '/'
            && (index == 0
                || characters[index - 1].is_whitespace()
                || matches!(
                    characters[index - 1],
                    '=' | ':' | '(' | '[' | '{' | '\'' | '"'
                ));
        if starts_path {
            result.push_str("<absolute-path>");
            index += 1;
            while index < characters.len()
                && !characters[index].is_whitespace()
                && !matches!(characters[index], ',' | ';' | ')' | ']' | '}' | '\'' | '"')
            {
                index += 1;
            }
        } else {
            result.push(characters[index]);
            index += 1;
        }
    }
    result
}

fn safe_detail_identity(detail: &str) -> String {
    let redacted = redact_absolute_paths(detail);
    let mut result = String::with_capacity(redacted.len());
    let mut in_digits = false;
    for character in redacted.chars().take(4096) {
        if character.is_ascii_digit() {
            if !in_digits {
                result.push('#');
                in_digits = true;
            }
        } else {
            in_digits = false;
            result.push(character);
        }
    }
    result.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub fn snapshot_from_output(output: &AdapterOutput) -> Value {
    json!({
        "repositoryTreeDigest":output.snapshot_input.repository_tree_digest,
        "vcsRevision":output.snapshot_input.vcs_revision,
        "dirty":output.snapshot_input.dirty,
        "buildModelDigest":output.snapshot_input.build_model_digest,
        "buildConfigurationDigest":output.snapshot_input.build_configuration_digest,
        "dependencyGraphDigest":output.snapshot_input.dependency_graph_digest,
        "generatedSourcesManifestDigest":output.snapshot_input.generated_sources_manifest_digest,
    })
}

pub fn semantic_output_digest(output: &AdapterOutput) -> Result<String> {
    let mut value = serde_json::to_value(output)?;
    if let Some(object) = value.as_object_mut() {
        object.remove("cost");
        object.remove("outputDigest");
    }
    if let Some(impact) = value.get_mut("impact").and_then(Value::as_object_mut) {
        impact.remove("queryMicros");
    }
    canonical_hash(&value)
}

pub fn semantic_facts_digest(output: &AdapterOutput) -> Result<String> {
    canonical_hash(&json!({
        "adapter":output.adapter,
        "snapshotInput":output.snapshot_input,
        "capabilityDescriptors":output.capability_descriptors,
        "entities":output.entities,
        "occurrences":output.occurrences,
        "facts":output.facts,
        "boundaries":output.boundaries,
        "compilerReceipt":output.compiler_receipt,
    }))
}

#[derive(Debug, Clone)]
pub struct CacheKey {
    pub digest: String,
    pub inputs: Value,
}

impl CacheKey {
    #[allow(clippy::too_many_arguments)]
    pub fn exact(
        repository_tree_digest: &str,
        vcs_revision: Option<&str>,
        dirty: bool,
        compilation: &str,
        adapter_binary_digest: &str,
        adapter_version: &str,
        worker_distribution: Value,
        worker_capabilities: Value,
        semantic_input_manifest: &Value,
        semantic_input_manifest_hash: &str,
    ) -> Result<Self> {
        let recomputed_manifest_hash = canonical_hash(semantic_input_manifest)?;
        if recomputed_manifest_hash != semantic_input_manifest_hash {
            bail!("live semantic input manifest hash is not canonical or does not match its body");
        }
        require_digest(repository_tree_digest)?;
        require_digest(adapter_binary_digest)?;
        require_digest(semantic_input_manifest_hash)?;
        require_distribution_identity(&worker_distribution)?;
        let inputs = json!({
            "schema":CACHE_INPUT_SCHEMA,
            "repositoryTreeDigest":repository_tree_digest,
            "vcsRevision":vcs_revision,
            "dirty":dirty,
            "compilation":compilation,
            "adapter":{
                "id":"codeclew.kotlin-k2",
                "version":adapter_version,
                "binaryDigest":adapter_binary_digest,
            },
            "workerDistribution":worker_distribution,
            "workerCapabilities":worker_capabilities,
            "semanticInputManifestHash":semantic_input_manifest_hash,
            "semanticInputManifest":semantic_input_manifest,
        });
        Ok(Self {
            digest: canonical_hash(&inputs)?,
            inputs,
        })
    }
}

fn require_distribution_identity(value: &Value) -> Result<()> {
    for field in ["treeHash", "buildInputDigest", "pluginFingerprint"] {
        require_digest(
            value
                .get(field)
                .and_then(Value::as_str)
                .with_context(|| format!("trusted worker distribution misses {field}"))?,
        )?;
    }
    Ok(())
}

fn agent_graph_normalization_digest(adapter_id: &str, adapter_version: &str) -> Result<String> {
    if adapter_id.is_empty() || adapter_version.is_empty() {
        bail!("agent graph normalization identity must be non-empty");
    }
    canonical_hash(&json!({
        "contract":AGENT_GRAPH_NORMALIZATION_CONTRACT,
        "adapterId":adapter_id,
        "adapterVersion":adapter_version,
        "projection":"BOUNDED_REVERSE_RESOLVED_RELATIONS",
    }))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AgentCacheSelector {
    schema: String,
    repository_tree_digest: String,
    vcs_revision: Option<String>,
    dirty: bool,
    compilation: String,
    adapter_id: String,
    adapter_version: String,
    normalization_contract_digest: String,
    build_state_seed_digest: Option<String>,
}

impl AgentCacheSelector {
    #[allow(clippy::too_many_arguments)]
    fn exact(
        repository_tree_digest: &str,
        vcs_revision: Option<&str>,
        dirty: bool,
        compilation: &str,
        adapter_id: &str,
        adapter_version: &str,
        build_state_seed_digest: Option<&str>,
    ) -> Result<Self> {
        require_digest(repository_tree_digest)?;
        if compilation.is_empty() || adapter_id.is_empty() || adapter_version.is_empty() {
            bail!("agent cache selector identity must be non-empty");
        }
        if let Some(digest) = build_state_seed_digest {
            require_digest(digest)?;
        }
        Ok(Self {
            schema: AGENT_CACHE_SELECTOR_SCHEMA.to_owned(),
            repository_tree_digest: repository_tree_digest.to_owned(),
            vcs_revision: vcs_revision.map(str::to_owned),
            dirty,
            compilation: compilation.to_owned(),
            adapter_id: adapter_id.to_owned(),
            adapter_version: adapter_version.to_owned(),
            normalization_contract_digest: agent_graph_normalization_digest(
                adapter_id,
                adapter_version,
            )?,
            build_state_seed_digest: build_state_seed_digest.map(str::to_owned),
        })
    }

    fn verify(&self) -> Result<()> {
        if self.schema != AGENT_CACHE_SELECTOR_SCHEMA
            || self.compilation.is_empty()
            || self.adapter_id.is_empty()
            || self.adapter_version.is_empty()
        {
            bail!("agent cache selector schema or identity mismatch");
        }
        require_digest(&self.repository_tree_digest)?;
        require_digest(&self.normalization_contract_digest)?;
        if self.normalization_contract_digest
            != agent_graph_normalization_digest(&self.adapter_id, &self.adapter_version)?
        {
            bail!("agent cache selector normalization contract mismatch");
        }
        if let Some(digest) = &self.build_state_seed_digest {
            require_digest(digest)?;
        }
        Ok(())
    }

    fn digest(&self) -> Result<String> {
        self.verify()?;
        canonical_hash(self)
    }

    fn matches_inputs(&self, inputs: &Value) -> bool {
        inputs.get("repositoryTreeDigest").and_then(Value::as_str)
            == Some(self.repository_tree_digest.as_str())
            && inputs.get("vcsRevision").and_then(Value::as_str) == self.vcs_revision.as_deref()
            && inputs.get("dirty").and_then(Value::as_bool) == Some(self.dirty)
            && inputs.get("compilation").and_then(Value::as_str) == Some(self.compilation.as_str())
            && inputs.pointer("/adapter/id").and_then(Value::as_str)
                == Some(self.adapter_id.as_str())
            && inputs.pointer("/adapter/version").and_then(Value::as_str)
                == Some(self.adapter_version.as_str())
            && match self.build_state_seed_digest.as_deref() {
                Some(seed) => {
                    inputs
                        .pointer("/semanticInputManifest/buildState/seedDigest")
                        .and_then(Value::as_str)
                        == Some(seed)
                }
                None => {
                    inputs
                        .pointer("/semanticInputManifest/buildState/mode")
                        .and_then(Value::as_str)
                        == Some("LEGACY_REPOSITORY_OWNED")
                }
            }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AgentGraphEntity {
    eligible_seed: bool,
    incident: bool,
    value: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AgentGraphShard {
    schema: String,
    bucket: String,
    source_semantic_object_digest: String,
    normalization_contract_digest: String,
    entities: BTreeMap<String, AgentGraphEntity>,
    aliases: BTreeMap<String, Vec<String>>,
    incoming: BTreeMap<String, Vec<Value>>,
    keyed_boundaries: BTreeMap<String, Vec<Value>>,
    object_digest: String,
}

impl AgentGraphShard {
    fn seal(&mut self) -> Result<()> {
        self.object_digest.clear();
        self.object_digest = canonical_hash(self)?;
        Ok(())
    }

    fn verify(
        &self,
        expected_bucket: &str,
        source_semantic_object_digest: &str,
        normalization_contract_digest: &str,
    ) -> Result<()> {
        if self.schema != AGENT_GRAPH_SHARD_SCHEMA
            || self.bucket != expected_bucket
            || self.source_semantic_object_digest != source_semantic_object_digest
            || self.normalization_contract_digest != normalization_contract_digest
            || !valid_agent_graph_bucket(&self.bucket)
        {
            bail!("agent graph shard identity mismatch");
        }
        require_digest(&self.source_semantic_object_digest)?;
        require_digest(&self.normalization_contract_digest)?;
        require_digest(&self.object_digest)?;
        let expected = self.object_digest.clone();
        let mut projection = self.clone();
        projection.object_digest.clear();
        if canonical_hash(&projection)? != expected {
            bail!("agent graph shard digest mismatch");
        }

        for key in self
            .entities
            .keys()
            .chain(self.aliases.keys())
            .chain(self.incoming.keys())
            .chain(self.keyed_boundaries.keys())
        {
            if key.is_empty() || agent_graph_bucket(key) != self.bucket {
                bail!("agent graph shard contains a key in the wrong bucket");
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AgentGraphShardRef {
    bucket: String,
    object_digest: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AgentGraphManifest {
    schema: String,
    source_kind: String,
    selector_digest: String,
    source_semantic_cache_key: String,
    source_semantic_object_digest: String,
    projection_authority: Value,
    normalization_contract: String,
    normalization_contract_digest: String,
    global_boundaries: Vec<Value>,
    project_boundary_count: u64,
    project_boundary_set_digest: String,
    boundary_kind_summary: BTreeMap<String, u64>,
    shards: Vec<AgentGraphShardRef>,
    object_digest: String,
}

impl AgentGraphManifest {
    fn seal(&mut self) -> Result<()> {
        self.object_digest.clear();
        self.object_digest = canonical_hash(self)?;
        Ok(())
    }

    fn verify(&self) -> Result<()> {
        if self.schema != AGENT_GRAPH_MANIFEST_SCHEMA
            || self.source_kind != "SEMANTIC_CACHE_OBJECT"
            || self.normalization_contract != AGENT_GRAPH_NORMALIZATION_CONTRACT
        {
            bail!("agent graph manifest schema or source identity mismatch");
        }
        for digest in [
            &self.selector_digest,
            &self.source_semantic_cache_key,
            &self.source_semantic_object_digest,
            &self.normalization_contract_digest,
            &self.project_boundary_set_digest,
            &self.object_digest,
        ] {
            require_digest(digest)?;
        }
        let expected = self.object_digest.clone();
        let mut projection = self.clone();
        projection.object_digest.clear();
        if canonical_hash(&projection)? != expected {
            bail!("agent graph manifest digest mismatch");
        }
        verify_projection_authority(&self.projection_authority)?;
        let mut previous: Option<&str> = None;
        for shard in &self.shards {
            if !valid_agent_graph_bucket(&shard.bucket)
                || previous.is_some_and(|value| value >= shard.bucket.as_str())
            {
                bail!("agent graph manifest has duplicate or unordered shard buckets");
            }
            require_digest(&shard.object_digest)?;
            previous = Some(&shard.bucket);
        }
        let mut previous_global: Option<String> = None;
        for boundary in &self.global_boundaries {
            let digest = canonical_hash(boundary)?;
            if previous_global
                .as_ref()
                .is_some_and(|previous| previous >= &digest)
            {
                bail!("agent graph global boundaries are not strictly digest-ordered");
            }
            previous_global = Some(digest);
        }
        let summarized_boundaries =
            self.boundary_kind_summary
                .values()
                .try_fold(0u64, |count, value| {
                    if *value == 0 {
                        bail!("agent graph boundary-kind summary contains a zero count");
                    }
                    Ok::<_, anyhow::Error>(count.saturating_add(*value))
                })?;
        if self
            .boundary_kind_summary
            .keys()
            .any(|kind| kind.is_empty())
        {
            bail!("agent graph boundary-kind summary is malformed or unordered");
        }
        if summarized_boundaries != self.project_boundary_count
            || self.global_boundaries.len() as u64 > self.project_boundary_count
        {
            bail!("agent graph project boundary count differs from by-kind summary");
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AgentCacheCatalog {
    schema: String,
    selector_digest: String,
    selector: AgentCacheSelector,
    semantic_cache_key: String,
    semantic_object_digest: String,
    agent_graph_manifest_digest: String,
    normalization_contract_digest: String,
    record_digest: String,
}

impl AgentCacheCatalog {
    fn new(
        selector: AgentCacheSelector,
        semantic_object: &SemanticCacheObject,
        manifest: &AgentGraphManifest,
    ) -> Result<Self> {
        let selector_digest = selector.digest()?;
        if manifest.selector_digest != selector_digest
            || manifest.source_semantic_cache_key != semantic_object.cache_key
            || manifest.source_semantic_object_digest != semantic_object.object_digest
            || manifest.normalization_contract_digest != selector.normalization_contract_digest
        {
            bail!("agent graph manifest differs from its catalog authority");
        }
        let mut record = Self {
            schema: AGENT_CACHE_CATALOG_SCHEMA.to_owned(),
            selector_digest,
            selector,
            semantic_cache_key: semantic_object.cache_key.clone(),
            semantic_object_digest: semantic_object.object_digest.clone(),
            agent_graph_manifest_digest: manifest.object_digest.clone(),
            normalization_contract_digest: manifest.normalization_contract_digest.clone(),
            record_digest: String::new(),
        };
        record.record_digest = canonical_hash(&record)?;
        Ok(record)
    }

    fn verify(&self, expected_selector: &AgentCacheSelector) -> Result<()> {
        if self.schema != AGENT_CACHE_CATALOG_SCHEMA
            || &self.selector != expected_selector
            || self.selector_digest != expected_selector.digest()?
            || self.normalization_contract_digest != expected_selector.normalization_contract_digest
        {
            bail!("agent cache catalog selector mismatch");
        }
        for digest in [
            &self.semantic_cache_key,
            &self.semantic_object_digest,
            &self.agent_graph_manifest_digest,
            &self.normalization_contract_digest,
            &self.record_digest,
        ] {
            require_digest(digest)?;
        }
        let expected = self.record_digest.clone();
        let mut projection = self.clone();
        projection.record_digest.clear();
        if canonical_hash(&projection)? != expected {
            bail!("agent cache catalog digest mismatch");
        }
        Ok(())
    }
}

fn valid_agent_graph_bucket(value: &str) -> bool {
    value.len() == AGENT_GRAPH_BUCKET_HEX_DIGITS
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn agent_graph_bucket(value: &str) -> String {
    hash_bytes(value.as_bytes())[7..7 + AGENT_GRAPH_BUCKET_HEX_DIGITS].to_owned()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SemanticCacheObject {
    schema: String,
    cache_key: String,
    inputs: Value,
    k2_validated: bool,
    semantic_output_digest: String,
    semantic_facts_digest: String,
    adapter_output: AdapterOutput,
    evidence_core: CoreBindingSummary,
    object_digest: String,
}

impl SemanticCacheObject {
    fn new(key: &CacheKey, output: &AdapterOutput, core: &CoreBindingSummary) -> Result<Self> {
        ensure_compiler_validated(output)?;
        let mut object = Self {
            schema: CACHE_SCHEMA.to_owned(),
            cache_key: key.digest.clone(),
            inputs: key.inputs.clone(),
            k2_validated: true,
            semantic_output_digest: semantic_output_digest(output)?,
            semantic_facts_digest: semantic_facts_digest(output)?,
            adapter_output: output.clone(),
            evidence_core: core.clone(),
            object_digest: String::new(),
        };
        object.object_digest = canonical_hash(&object)?;
        Ok(object)
    }

    fn verify_integrity(&self, key: &CacheKey) -> Result<()> {
        if self.schema != CACHE_SCHEMA
            || self.cache_key != key.digest
            || self.inputs != key.inputs
            || !self.k2_validated
        {
            bail!("semantic cache object identity differs from exact live inputs");
        }
        let expected = self.object_digest.clone();
        let mut projection = self.clone();
        projection.object_digest.clear();
        if canonical_hash(&projection)? != expected {
            bail!("semantic cache object digest mismatch");
        }
        self.adapter_output.verify_seal()?;
        ensure_cache_payload_matches_key(&self.adapter_output, key)?;
        if self.adapter_output.snapshot_input.repository_tree_digest
            != key.inputs["repositoryTreeDigest"]
            || self.adapter_output.adapter.binary_digest != key.inputs["adapter"]["binaryDigest"]
            || semantic_output_digest(&self.adapter_output)? != self.semantic_output_digest
            || semantic_facts_digest(&self.adapter_output)? != self.semantic_facts_digest
        {
            bail!("semantic cache payload is stale for the live snapshot or adapter");
        }
        Ok(())
    }

    fn verify(&self, key: &CacheKey) -> Result<CoreBindingSummary> {
        self.verify_integrity(key)?;
        let rebound = validate_kotlin_core_binding(&self.adapter_output)?;
        if !same_core_semantics(&rebound, &self.evidence_core) {
            bail!("semantic cache evidence-core binding differs after revalidation");
        }
        Ok(rebound)
    }
}

fn ensure_cache_payload_matches_key(output: &AdapterOutput, key: &CacheKey) -> Result<()> {
    let receipt = output
        .compiler_receipt
        .get("providerPayload")
        .and_then(Value::as_object)
        .context("compiler receipt providerPayload is absent")?;
    if output.adapter.adapter_id != key.inputs["adapter"]["id"]
        || output.adapter.version != key.inputs["adapter"]["version"]
        || output.snapshot_input.vcs_revision.as_deref() != key.inputs["vcsRevision"].as_str()
        || output.snapshot_input.dirty
            != key.inputs["dirty"]
                .as_bool()
                .unwrap_or(!output.snapshot_input.dirty)
        || !output
            .snapshot_input
            .targets
            .iter()
            .any(|target| target.get("targetId") == key.inputs.get("compilation"))
        || receipt.get("semanticInputManifestHash") != key.inputs.get("semanticInputManifestHash")
        || receipt.get("semanticInputManifest") != key.inputs.get("semanticInputManifest")
        || receipt.get("trustedWorkerDistribution") != key.inputs.get("workerDistribution")
        || receipt.get("adapterBinaryDigest") != key.inputs["adapter"].get("binaryDigest")
        || receipt.get("analyzerCompilerVersion")
            != key.inputs["semanticInputManifest"].get("analyzerCompilerVersion")
    {
        bail!("semantic cache compiler receipt is not bound to the exact live cache inputs");
    }
    Ok(())
}

fn same_core_semantics(left: &CoreBindingSummary, right: &CoreBindingSummary) -> bool {
    left.schema == right.schema
        && left.snapshot_id == right.snapshot_id
        && left.bundle_digest == right.bundle_digest
        && left.capability_count == right.capability_count
        && left.batch_count == right.batch_count
        && left.fact_count == right.fact_count
        && left.obligation_graph_count == right.obligation_graph_count
        && left.impact_receipt_count == right.impact_receipt_count
        && left.canonical_bundle_bytes == right.canonical_bundle_bytes
}

fn ensure_compiler_validated(output: &AdapterOutput) -> Result<()> {
    if output
        .compiler_receipt
        .pointer("/providerPayload/k2Validated")
        .and_then(Value::as_bool)
        != Some(true)
        || output
            .compiler_receipt
            .get("status")
            .and_then(Value::as_str)
            != Some("ACCEPTED")
        || output.compiler_receipt.get("grade").and_then(Value::as_str) != Some("COMPILER_CHECKED")
    {
        bail!("semantic result has no explicit successful K2 validation");
    }
    Ok(())
}

#[derive(Default)]
struct AgentGraphShardRows {
    entities: BTreeMap<String, AgentGraphEntity>,
    aliases: BTreeMap<String, Vec<String>>,
    incoming: BTreeMap<String, Vec<Value>>,
    keyed_boundaries: BTreeMap<String, Vec<Value>>,
}

fn explicit_boundary_query_keys(boundary: &Value) -> Option<Vec<String>> {
    let applicability = boundary.get("applicability")?.as_object()?;
    let effect = applicability.get("effect")?.as_str()?;
    if applicability.get("schema")?.as_str()? != "codeclew.boundary-applicability/0.1"
        || applicability.get("locality")?.as_str()? != "EXACT"
        || applicability.get("direction")?.as_str()? != "INCOMING"
        || !matches!(
            effect,
            BOUNDARY_EFFECT_TOPOLOGY
                | BOUNDARY_EFFECT_ATTRIBUTE
                | BOUNDARY_EFFECT_BUILD_FIDELITY
                | BOUNDARY_EFFECT_OUT_OF_SCOPE
        )
        || (effect == BOUNDARY_EFFECT_OUT_OF_SCOPE
            && applicability.get("entityScope")?.as_str()? != IMPACT_QUERY_ENTITY_SCOPE)
    {
        return None;
    }
    let strings = |field: &str| {
        applicability
            .get(field)?
            .as_array()?
            .iter()
            .map(|row| {
                row.as_str()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
            })
            .collect::<Option<BTreeSet<_>>>()
    };
    strings("operations")?;
    strings("ownerQueryKeys")?;
    let keys = strings("targetQueryKeys")?;
    (!keys.is_empty()).then(|| keys.into_iter().map(str::to_owned).collect())
}

type CanonicalBoundarySet = (Vec<(String, Value)>, String, BTreeMap<String, u64>);

fn canonical_boundary_set(boundaries: &[Value]) -> Result<CanonicalBoundarySet> {
    let mut members = BTreeMap::new();
    for boundary in boundaries {
        if !boundary.is_object() {
            bail!("agent graph boundary must be an object");
        }
        members
            .entry(canonical_hash(boundary)?)
            .or_insert_with(|| boundary.clone());
    }
    let members = members.into_iter().collect::<Vec<_>>();
    let set_digest = canonical_hash(&json!({
        "schema":AGENT_GRAPH_BOUNDARY_SET_SCHEMA,
        "members":members.iter().map(|(digest, boundary)| json!({
            "boundaryDigest":digest,
            "boundary":boundary,
        })).collect::<Vec<_>>(),
    }))?;
    let mut kinds = BTreeMap::<String, u64>::new();
    for (_, boundary) in &members {
        let kind = boundary
            .get("kindUri")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .unwrap_or("codeclew.boundary/unknown/1");
        *kinds.entry(kind.to_owned()).or_default() += 1;
    }
    Ok((members, set_digest, kinds))
}

fn projection_authority(output: &AdapterOutput, compilation: &str) -> Result<Value> {
    let receipt = &output.compiler_receipt;
    Ok(json!({
        "schema":"codeclew.kotlin-agent-projection-authority/0.1",
        "adapter":{
            "id":output.adapter.adapter_id,
            "version":output.adapter.version,
            "binaryDigest":output.adapter.binary_digest,
            "languageId":output.adapter.language_id,
        },
        "snapshot":{
            "repositoryTreeDigest":output.snapshot_input.repository_tree_digest,
            "vcsRevision":output.snapshot_input.vcs_revision,
            "dirty":output.snapshot_input.dirty,
            "selectedCompilation":compilation,
            "targets":output.snapshot_input.targets,
        },
        "compilerReceipt":{
            "method":receipt.get("method"),
            "status":receipt.get("status"),
            "grade":receipt.get("grade"),
            "snapshotTreeDigest":receipt.get("snapshotTreeDigest"),
            "k2Validated":receipt.pointer("/providerPayload/k2Validated"),
            "analyzerCompilerVersion":receipt.pointer("/providerPayload/analyzerCompilerVersion"),
            "adapterBinaryDigest":receipt.pointer("/providerPayload/adapterBinaryDigest"),
        },
        "semanticOutputDigest":semantic_output_digest(output)?,
        "semanticFactsDigest":semantic_facts_digest(output)?,
    }))
}

fn verify_projection_authority(value: &Value) -> Result<()> {
    if value.get("schema").and_then(Value::as_str)
        != Some("codeclew.kotlin-agent-projection-authority/0.1")
        || value
            .pointer("/compilerReceipt/status")
            .and_then(Value::as_str)
            != Some("ACCEPTED")
        || value
            .pointer("/compilerReceipt/grade")
            .and_then(Value::as_str)
            != Some("COMPILER_CHECKED")
        || value
            .pointer("/compilerReceipt/k2Validated")
            .and_then(Value::as_bool)
            != Some(true)
        || value
            .pointer("/snapshot/targets")
            .and_then(Value::as_array)
            .is_none()
        || value
            .pointer("/snapshot/selectedCompilation")
            .and_then(Value::as_str)
            .is_none_or(str::is_empty)
    {
        bail!("agent graph projection authority is incomplete");
    }
    for pointer in [
        "/adapter/binaryDigest",
        "/snapshot/repositoryTreeDigest",
        "/compilerReceipt/snapshotTreeDigest",
        "/compilerReceipt/adapterBinaryDigest",
        "/semanticOutputDigest",
        "/semanticFactsDigest",
    ] {
        require_digest(
            value
                .pointer(pointer)
                .and_then(Value::as_str)
                .with_context(|| format!("projection authority misses {pointer}"))?,
        )?;
    }
    if value.pointer("/adapter/binaryDigest")
        != value.pointer("/compilerReceipt/adapterBinaryDigest")
        || value.pointer("/snapshot/repositoryTreeDigest")
            != value.pointer("/compilerReceipt/snapshotTreeDigest")
    {
        bail!("projection authority compiler identity differs from snapshot");
    }
    Ok(())
}

fn build_agent_graph_index(
    semantic_object: &SemanticCacheObject,
) -> Result<(AgentGraphManifest, Vec<AgentGraphShard>)> {
    let key = CacheKey {
        digest: semantic_object.cache_key.clone(),
        inputs: semantic_object.inputs.clone(),
    };
    semantic_object.verify_integrity(&key)?;
    let selector_digest = selector_from_semantic_object(semantic_object)?.digest()?;
    let output = &semantic_object.adapter_output;
    let normalization_contract_digest =
        agent_graph_normalization_digest(&output.adapter.adapter_id, &output.adapter.version)?;
    let compilation = key
        .inputs
        .get("compilation")
        .and_then(Value::as_str)
        .context("semantic cache key has no compilation")?;
    let (boundary_members, project_boundary_set_digest, boundary_kind_summary) =
        canonical_boundary_set(&output.boundaries)?;
    let project_boundary_count = boundary_members.len() as u64;

    let mut incident_entities = BTreeSet::new();
    let mut incoming = BTreeMap::<String, Vec<Value>>::new();
    let mut fact_ids = BTreeSet::new();
    for fact in &output.facts {
        if fact.get("truth").and_then(Value::as_str) != Some("TRUE") {
            continue;
        }
        let fact_id = fact
            .get("factId")
            .and_then(Value::as_str)
            .context("true semantic fact has no factId")?;
        let owner = fact
            .get("owner")
            .and_then(Value::as_str)
            .context("true semantic fact has no owner")?;
        let target = fact
            .get("target")
            .and_then(Value::as_str)
            .context("true semantic fact has no target")?;
        require_digest(fact_id)?;
        if owner.is_empty() || target.is_empty() || !fact_ids.insert(fact_id.to_owned()) {
            bail!("true semantic facts have empty endpoints or duplicate identities");
        }
        incident_entities.insert(owner.to_owned());
        incident_entities.insert(target.to_owned());
        incoming
            .entry(target.to_owned())
            .or_default()
            .push(fact.clone());
    }

    let mut entity_records = BTreeMap::new();
    let mut aliases = BTreeMap::<String, BTreeSet<String>>::new();
    for entity in &output.entities {
        let entity_id = entity
            .get("opaqueId")
            .and_then(Value::as_str)
            .context("semantic entity has no opaqueId")?;
        if entity_id.is_empty() {
            bail!("semantic entity has an empty opaqueId");
        }
        let eligible_seed = entity.get("resolution").and_then(Value::as_str) == Some("RESOLVED")
            && entity
                .get("primaryDefinition")
                .is_some_and(|definition| !definition.is_null());
        let record = AgentGraphEntity {
            eligible_seed,
            incident: incident_entities.contains(entity_id),
            value: entity.clone(),
        };
        if entity_records
            .insert(entity_id.to_owned(), record)
            .is_some()
        {
            bail!("semantic graph contains duplicate entity opaqueId");
        }
        if !eligible_seed {
            continue;
        }
        if let Some(alias) = entity
            .pointer("/languagePayload/compilerCallableId")
            .and_then(Value::as_str)
            .filter(|alias| !alias.is_empty())
        {
            aliases
                .entry(alias.to_owned())
                .or_default()
                .insert(entity_id.to_owned());
        }
        if matches!(
            entity
                .pointer("/languagePayload/declarationKind")
                .and_then(Value::as_str),
            Some("CLASS" | "INTERFACE" | "OBJECT" | "TYPE_ALIAS")
        ) && let Some(alias) = entity
            .pointer("/languagePayload/compilerClassId")
            .and_then(Value::as_str)
            .filter(|alias| !alias.is_empty())
        {
            aliases
                .entry(alias.to_owned())
                .or_default()
                .insert(entity_id.to_owned());
        }
    }

    let mut rows = BTreeMap::<String, AgentGraphShardRows>::new();
    for (entity_id, record) in entity_records {
        rows.entry(agent_graph_bucket(&entity_id))
            .or_default()
            .entities
            .insert(entity_id, record);
    }
    for (alias, entity_ids) in aliases {
        rows.entry(agent_graph_bucket(&alias))
            .or_default()
            .aliases
            .insert(alias, entity_ids.into_iter().collect());
    }
    for (target, facts) in incoming {
        rows.entry(agent_graph_bucket(&target))
            .or_default()
            .incoming
            .insert(target, facts);
    }
    let mut global_boundaries = Vec::new();
    for (_, boundary) in boundary_members {
        if let Some(query_keys) = explicit_boundary_query_keys(&boundary) {
            for query_key in query_keys {
                rows.entry(agent_graph_bucket(&query_key))
                    .or_default()
                    .keyed_boundaries
                    .entry(query_key)
                    .or_default()
                    .push(boundary.clone());
            }
        } else {
            // Applicability that is absent, empty, or malformed is never
            // guessed. It remains a global topology boundary for every query.
            global_boundaries.push(boundary);
        }
    }

    let mut shards = Vec::new();
    let mut shard_refs = Vec::new();
    for bucket in rows.values_mut() {
        for boundaries in bucket.keyed_boundaries.values_mut() {
            boundaries.sort_by_key(|boundary| canonical_hash(boundary).unwrap_or_default());
        }
    }
    for (bucket, bucket_rows) in rows {
        let mut shard = AgentGraphShard {
            schema: AGENT_GRAPH_SHARD_SCHEMA.to_owned(),
            bucket: bucket.clone(),
            source_semantic_object_digest: semantic_object.object_digest.clone(),
            normalization_contract_digest: normalization_contract_digest.clone(),
            entities: bucket_rows.entities,
            aliases: bucket_rows.aliases,
            incoming: bucket_rows.incoming,
            keyed_boundaries: bucket_rows.keyed_boundaries,
            object_digest: String::new(),
        };
        shard.seal()?;
        shard.verify(
            &bucket,
            &semantic_object.object_digest,
            &normalization_contract_digest,
        )?;
        shard_refs.push(AgentGraphShardRef {
            bucket,
            object_digest: shard.object_digest.clone(),
        });
        shards.push(shard);
    }

    let mut manifest = AgentGraphManifest {
        schema: AGENT_GRAPH_MANIFEST_SCHEMA.to_owned(),
        source_kind: "SEMANTIC_CACHE_OBJECT".to_owned(),
        selector_digest,
        source_semantic_cache_key: semantic_object.cache_key.clone(),
        source_semantic_object_digest: semantic_object.object_digest.clone(),
        projection_authority: projection_authority(output, compilation)?,
        normalization_contract: AGENT_GRAPH_NORMALIZATION_CONTRACT.to_owned(),
        normalization_contract_digest,
        global_boundaries,
        project_boundary_count,
        project_boundary_set_digest,
        boundary_kind_summary,
        shards: shard_refs,
        object_digest: String::new(),
    };
    manifest.seal()?;
    manifest.verify()?;
    Ok((manifest, shards))
}

/// Kotlin-owned fail-closed entrypoint to the frozen evidence-core binding.
/// A syntactically accepted compiler receipt is not sufficient: the retained
/// K2 validation bit must be explicit before shared validation is attempted.
pub(super) fn validate_kotlin_core_binding(output: &AdapterOutput) -> Result<CoreBindingSummary> {
    ensure_compiler_validated(output)?;
    validate_core_binding(output)
}

#[derive(Debug)]
pub struct AgentGraphSeed {
    pub entity_id: String,
    pub entity: Value,
}

/// A bounded reverse index derived from one verified semantic cache object.
pub struct AgentGraphIndex {
    shards: PathBuf,
    manifest: AgentGraphManifest,
}

impl AgentGraphIndex {
    pub fn projection_authority(&self) -> &Value {
        &self.manifest.projection_authority
    }

    pub fn entity(&self, entity_id: &str) -> Result<Option<Value>> {
        let Some(shard) = self.load_bucket(entity_id)? else {
            return Ok(None);
        };
        Ok(shard
            .entities
            .get(entity_id)
            .map(|record| record.value.clone()))
    }

    pub fn incoming_facts(&self, target: &str) -> Result<Vec<Value>> {
        let Some(shard) = self.load_bucket(target)? else {
            return Ok(Vec::new());
        };
        Ok(shard.incoming.get(target).cloned().unwrap_or_default())
    }

    pub fn boundaries_for_query_key(&self, query_key: &str) -> Result<Vec<Value>> {
        let mut boundaries = self.manifest.global_boundaries.clone();
        if let Some(shard) = self.load_bucket(query_key)? {
            boundaries.extend(
                shard
                    .keyed_boundaries
                    .get(query_key)
                    .cloned()
                    .unwrap_or_default(),
            );
        }
        let mut unique = BTreeMap::new();
        for boundary in boundaries {
            unique.insert(canonical_hash(&boundary)?, boundary);
        }
        Ok(unique.into_values().collect())
    }

    pub fn project_boundary_summary(&self) -> Value {
        json!({
            "count":self.manifest.project_boundary_count,
            "setDigest":self.manifest.project_boundary_set_digest,
            "globalCount":self.manifest.global_boundaries.len(),
            "byKind":&self.manifest.boundary_kind_summary,
        })
    }

    pub fn resolve_seed(&self, requested: &str) -> Result<AgentGraphSeed> {
        if requested.is_empty() {
            bail!("requested seed is empty");
        }
        let requested_shard = self
            .load_bucket(requested)?
            .context("requested seed does not resolve to a source-defined entity")?;
        if let Some(record) = requested_shard
            .entities
            .get(requested)
            .filter(|record| record.eligible_seed)
        {
            if !record.incident {
                bail!("requested seed has no incident compiler relation");
            }
            return Ok(AgentGraphSeed {
                entity_id: requested.to_owned(),
                entity: record.value.clone(),
            });
        }
        let alias = requested_shard
            .aliases
            .get(requested)
            .context("requested seed does not resolve to a source-defined entity")?;
        let [entity_id] = alias.as_slice() else {
            bail!("requested seed is ambiguous across resolved source entities");
        };
        let entity_shard = self
            .load_bucket(entity_id)?
            .context("agent graph alias references a missing entity shard")?;
        let record = entity_shard
            .entities
            .get(entity_id)
            .filter(|record| record.eligible_seed)
            .context("agent graph alias references an ineligible or missing entity")?;
        if !record.incident {
            bail!("requested seed has no incident compiler relation");
        }
        Ok(AgentGraphSeed {
            entity_id: entity_id.clone(),
            entity: record.value.clone(),
        })
    }

    fn load_bucket(&self, key: &str) -> Result<Option<AgentGraphShard>> {
        let bucket = agent_graph_bucket(key);
        let Some(shard_ref) = self
            .manifest
            .shards
            .binary_search_by(|candidate| candidate.bucket.as_str().cmp(bucket.as_str()))
            .ok()
            .map(|index| &self.manifest.shards[index])
        else {
            return Ok(None);
        };
        Ok(Some(self.load_shard(shard_ref)?))
    }

    fn load_shard(&self, shard_ref: &AgentGraphShardRef) -> Result<AgentGraphShard> {
        let shard: AgentGraphShard = read_canonical_content_object(
            &self.shards,
            &shard_ref.object_digest,
            "agent graph shard",
        )?;
        shard.verify(
            &shard_ref.bucket,
            &self.manifest.source_semantic_object_digest,
            &self.manifest.normalization_contract_digest,
        )?;
        Ok(shard)
    }
}

#[derive(Debug)]
pub struct CacheHit {
    pub output: AdapterOutput,
    pub bytes_read: u64,
    pub read_micros: u64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CacheIndexTelemetry {
    pub legacy_objects_scanned: u64,
    pub legacy_bytes_scanned: u64,
    pub migration_bytes_written: u64,
}

pub struct AgentGraphLookup {
    pub index: AgentGraphIndex,
    pub bytes_read: u64,
    pub read_micros: u64,
    pub index_telemetry: CacheIndexTelemetry,
}

struct CatalogResolution {
    catalog: AgentCacheCatalog,
    catalog_bytes_read: u64,
    telemetry: CacheIndexTelemetry,
}

#[derive(Debug)]
pub enum CacheLookup {
    Miss { read_micros: u64 },
    Hit(Box<CacheHit>),
}

#[derive(Debug)]
pub struct CachePublication {
    pub bytes_written: u64,
    pub write_micros: u64,
}

pub struct SemanticCache {
    root: PathBuf,
    compiler_index: PathBuf,
    objects: PathBuf,
    agent_catalogs: PathBuf,
    agent_graph_manifests: PathBuf,
    agent_graph_shards: PathBuf,
}

impl SemanticCache {
    pub fn open(path: &Path, repository: &Path) -> Result<Self> {
        if !path.is_absolute() {
            bail!("K1 state root must be absolute");
        }
        let metadata = match std::fs::symlink_metadata(path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == ErrorKind::NotFound => {
                let mut builder = std::fs::DirBuilder::new();
                #[cfg(unix)]
                {
                    use std::os::unix::fs::DirBuilderExt;
                    builder.mode(0o700);
                }
                builder
                    .create(path)
                    .context("create private K1 state root")?;
                std::fs::symlink_metadata(path)?
            }
            Err(error) => return Err(error.into()),
        };
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            bail!("K1 state root must be a regular non-symlink directory");
        }
        require_private_directory(&metadata, "K1 state root")?;
        let root = path.canonicalize()?;
        if root != path {
            bail!("K1 state root must be canonical and have no symlinked ancestor");
        }
        let repository = repository.canonicalize()?;
        if root.starts_with(&repository) || repository.starts_with(&root) {
            bail!("K1 state root must be outside and must not contain the source checkout");
        }
        let compiler_index = root.join("compiler-index");
        create_private_checked_directory(&compiler_index)?;
        let cache_root = root.join("semantic-cache");
        create_checked_directory(&cache_root)?;
        let objects = cache_root.join("sha256");
        create_checked_directory(&objects)?;
        let agent_catalog_root = cache_root.join("agent-query-catalog");
        create_checked_directory(&agent_catalog_root)?;
        let agent_catalogs = agent_catalog_root.join("sha256");
        create_checked_directory(&agent_catalogs)?;
        let agent_graph_root = cache_root.join("agent-graph");
        create_checked_directory(&agent_graph_root)?;
        let agent_graph_manifest_root = agent_graph_root.join("manifests");
        create_checked_directory(&agent_graph_manifest_root)?;
        let agent_graph_manifests = agent_graph_manifest_root.join("sha256");
        create_checked_directory(&agent_graph_manifests)?;
        let agent_graph_shard_root = agent_graph_root.join("shards");
        create_checked_directory(&agent_graph_shard_root)?;
        let agent_graph_shards = agent_graph_shard_root.join("sha256");
        create_checked_directory(&agent_graph_shards)?;
        Ok(Self {
            root,
            compiler_index,
            objects,
            agent_catalogs,
            agent_graph_manifests,
            agent_graph_shards,
        })
    }

    pub fn canonical_root(&self) -> &Path {
        &self.root
    }

    pub fn compiler_index_root(&self) -> &Path {
        &self.compiler_index
    }

    pub fn root_digest(&self) -> String {
        // Never expose or semantically bind a volatile absolute state path.
        hash_bytes(b"external-k1-state-root")
    }

    fn object_path(&self, key: &CacheKey) -> Result<PathBuf> {
        content_object_path(&self.objects, &key.digest)
    }

    fn catalog_path(&self, selector: &AgentCacheSelector) -> Result<PathBuf> {
        content_object_path(&self.agent_catalogs, &selector.digest()?)
    }

    pub fn lookup(&self, key: &CacheKey) -> Result<CacheLookup> {
        let started = Instant::now();
        let path = self.object_path(key)?;
        let metadata = match std::fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == ErrorKind::NotFound => {
                return Ok(CacheLookup::Miss {
                    read_micros: started.elapsed().as_micros() as u64,
                });
            }
            Err(error) => return Err(error.into()),
        };
        if !metadata.is_file() || metadata.file_type().is_symlink() {
            bail!("semantic cache object is not a regular non-symlink file");
        }
        if path
            .parent()
            .and_then(|parent| parent.canonicalize().ok())
            .as_deref()
            != Some(self.objects.as_path())
        {
            bail!("semantic cache object escaped its content-addressed directory");
        }
        let bytes = read_immutable_regular(&path)?;
        let object: SemanticCacheObject = serde_json::from_slice(&bytes)
            .context("semantic cache object is invalid or truncated JSON")?;
        if canonical_bytes(&object)? != bytes {
            bail!("semantic cache object is not exact canonical JSON");
        }
        object.verify(key)?;
        Ok(CacheLookup::Hit(Box::new(CacheHit {
            output: object.adapter_output,
            bytes_read: bytes.len() as u64,
            read_micros: started.elapsed().as_micros() as u64,
        })))
    }

    #[allow(clippy::too_many_arguments)]
    pub fn lookup_agent_graph_query(
        &self,
        repository_tree_digest: &str,
        vcs_revision: Option<&str>,
        dirty: bool,
        compilation: &str,
        adapter_id: &str,
        adapter_version: &str,
        build_state_seed_digest: Option<&str>,
    ) -> Result<Option<AgentGraphLookup>> {
        let started = Instant::now();
        let selector = AgentCacheSelector::exact(
            repository_tree_digest,
            vcs_revision,
            dirty,
            compilation,
            adapter_id,
            adapter_version,
            build_state_seed_digest,
        )?;
        let Some(resolved) = self.resolve_agent_catalog(&selector)? else {
            return Ok(None);
        };
        let (index, manifest_bytes) = self.load_agent_graph_manifest(
            &resolved.catalog.agent_graph_manifest_digest,
            Some(&resolved.catalog),
        )?;
        let bytes_read = resolved
            .catalog_bytes_read
            .saturating_add(resolved.telemetry.legacy_bytes_scanned)
            .saturating_add(manifest_bytes);
        Ok(Some(AgentGraphLookup {
            index,
            bytes_read,
            read_micros: started.elapsed().as_micros() as u64,
            index_telemetry: resolved.telemetry,
        }))
    }

    fn resolve_agent_catalog(
        &self,
        selector: &AgentCacheSelector,
    ) -> Result<Option<CatalogResolution>> {
        Ok(self
            .read_catalog(selector)?
            .map(|(catalog, bytes)| CatalogResolution {
                catalog,
                catalog_bytes_read: bytes,
                telemetry: CacheIndexTelemetry::default(),
            }))
    }

    fn read_catalog(
        &self,
        selector: &AgentCacheSelector,
    ) -> Result<Option<(AgentCacheCatalog, u64)>> {
        let path = self.catalog_path(selector)?;
        match std::fs::symlink_metadata(&path) {
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.into()),
            Ok(metadata) if !metadata.is_file() || metadata.file_type().is_symlink() => {
                bail!("agent cache catalog is not a regular non-symlink file")
            }
            Ok(_) => {}
        }
        let bytes = read_immutable_regular(&path)?;
        let catalog: AgentCacheCatalog = serde_json::from_slice(&bytes)
            .context("agent cache catalog is invalid or truncated JSON")?;
        if canonical_bytes(&catalog)? != bytes {
            bail!("agent cache catalog is not exact canonical JSON");
        }
        catalog.verify(selector)?;
        Ok(Some((catalog, bytes.len() as u64)))
    }

    fn load_agent_graph_manifest(
        &self,
        manifest_digest: &str,
        catalog: Option<&AgentCacheCatalog>,
    ) -> Result<(AgentGraphIndex, u64)> {
        let (manifest, bytes): (AgentGraphManifest, u64) = read_canonical_content_object_with_len(
            &self.agent_graph_manifests,
            manifest_digest,
            "agent graph manifest",
        )?;
        manifest.verify()?;
        if manifest.object_digest != manifest_digest {
            bail!("agent graph manifest filename differs from object digest");
        }
        if let Some(catalog) = catalog
            && (manifest.selector_digest != catalog.selector_digest
                || manifest.source_semantic_cache_key != catalog.semantic_cache_key
                || manifest.source_semantic_object_digest != catalog.semantic_object_digest
                || manifest.normalization_contract_digest != catalog.normalization_contract_digest
                || manifest.object_digest != catalog.agent_graph_manifest_digest)
        {
            bail!("agent graph manifest binding differs from cache catalog");
        }
        for shard in &manifest.shards {
            validate_content_object_exists(
                &self.agent_graph_shards,
                &shard.object_digest,
                "agent graph shard",
            )?;
        }
        Ok((
            AgentGraphIndex {
                shards: self.agent_graph_shards.clone(),
                manifest,
            },
            bytes,
        ))
    }

    fn publish_agent_graph(
        &self,
        semantic_object: &SemanticCacheObject,
    ) -> Result<(AgentGraphManifest, u64)> {
        let (manifest, shards) = build_agent_graph_index(semantic_object)?;
        let mut bytes_written = 0u64;
        for shard in &shards {
            bytes_written = bytes_written.saturating_add(publish_immutable_content_object(
                &self.agent_graph_shards,
                &shard.object_digest,
                shard,
                "agent graph shard",
            )?);
        }
        bytes_written = bytes_written.saturating_add(publish_immutable_content_object(
            &self.agent_graph_manifests,
            &manifest.object_digest,
            &manifest,
            "agent graph manifest",
        )?);
        self.load_agent_graph_manifest(&manifest.object_digest, None)?;
        Ok((manifest, bytes_written))
    }

    fn publish_catalog(
        &self,
        semantic_object: &SemanticCacheObject,
        manifest: &AgentGraphManifest,
    ) -> Result<u64> {
        let selector = selector_from_semantic_object(semantic_object)?;
        let catalog = AgentCacheCatalog::new(selector.clone(), semantic_object, manifest)?;
        let bytes = canonical_bytes(&catalog)?;
        let path = self.catalog_path(&selector)?;
        replace_canonical_catalog(&self.agent_catalogs, &path, &bytes)?;
        let Some((published, _)) = self.read_catalog(&selector)? else {
            bail!("agent cache catalog disappeared after publication");
        };
        if canonical_bytes(&published)? != bytes {
            bail!("agent cache catalog differs after atomic publication");
        }
        Ok(bytes.len() as u64)
    }

    pub fn publish(
        &self,
        key: &CacheKey,
        output: &AdapterOutput,
        core: &CoreBindingSummary,
    ) -> Result<CachePublication> {
        let started = Instant::now();
        let object = SemanticCacheObject::new(key, output, core)?;
        let bytes = canonical_bytes(&object)?;
        let target = self.object_path(key)?;
        let temporary = self.objects.join(format!(
            ".{}.{}.tmp",
            key.digest.strip_prefix("sha256:").unwrap_or("invalid"),
            std::process::id()
        ));
        let mut file = create_private_new(&temporary)?;
        let result = (|| -> Result<()> {
            file.write_all(&bytes)?;
            file.sync_all()?;
            match std::fs::hard_link(&temporary, &target) {
                Ok(()) => Ok(()),
                Err(error) if error.kind() == ErrorKind::AlreadyExists => {
                    match self.lookup(key)? {
                        CacheLookup::Hit(existing)
                            if existing.output.output_digest == output.output_digest =>
                        {
                            Ok(())
                        }
                        CacheLookup::Hit(_) => bail!("immutable semantic cache key collision"),
                        CacheLookup::Miss { .. } => {
                            bail!("semantic cache object disappeared during publication")
                        }
                    }
                }
                Err(error) => Err(error.into()),
            }
        })();
        drop(file);
        let _ = std::fs::remove_file(&temporary);
        result?;
        // A publication is not complete until the exact object revalidates.
        if !matches!(self.lookup(key)?, CacheLookup::Hit(_)) {
            bail!("semantic cache publication did not produce a verified object");
        }
        let (agent_graph_manifest, agent_graph_bytes) = self.publish_agent_graph(&object)?;
        let catalog_bytes = self.publish_catalog(&object, &agent_graph_manifest)?;
        Ok(CachePublication {
            bytes_written: (bytes.len() as u64)
                .saturating_add(agent_graph_bytes)
                .saturating_add(catalog_bytes),
            write_micros: started.elapsed().as_micros() as u64,
        })
    }
}

fn selector_from_semantic_object(object: &SemanticCacheObject) -> Result<AgentCacheSelector> {
    let inputs = &object.inputs;
    let repository_tree_digest = inputs
        .get("repositoryTreeDigest")
        .and_then(Value::as_str)
        .context("semantic cache inputs have no repository tree digest")?;
    let vcs_revision = match inputs.get("vcsRevision") {
        None | Some(Value::Null) => None,
        Some(Value::String(value)) => Some(value.as_str()),
        Some(_) => bail!("semantic cache VCS revision has invalid type"),
    };
    let dirty = inputs
        .get("dirty")
        .and_then(Value::as_bool)
        .context("semantic cache inputs have no dirty bit")?;
    let compilation = inputs
        .get("compilation")
        .and_then(Value::as_str)
        .context("semantic cache inputs have no compilation")?;
    let adapter_id = inputs
        .pointer("/adapter/id")
        .and_then(Value::as_str)
        .context("semantic cache inputs have no adapter id")?;
    let adapter_version = inputs
        .pointer("/adapter/version")
        .and_then(Value::as_str)
        .context("semantic cache inputs have no adapter version")?;
    let build_state_seed_digest = inputs
        .pointer("/semanticInputManifest/buildState/seedDigest")
        .and_then(Value::as_str);
    if build_state_seed_digest.is_none()
        && inputs
            .pointer("/semanticInputManifest/buildState/mode")
            .and_then(Value::as_str)
            != Some("LEGACY_REPOSITORY_OWNED")
    {
        bail!("semantic cache inputs have no supported build-state selector");
    }
    let selector = AgentCacheSelector::exact(
        repository_tree_digest,
        vcs_revision,
        dirty,
        compilation,
        adapter_id,
        adapter_version,
        build_state_seed_digest,
    )?;
    if !selector.matches_inputs(inputs) {
        bail!("derived agent cache selector differs from semantic cache inputs");
    }
    Ok(selector)
}

fn content_object_path(directory: &Path, digest: &str) -> Result<PathBuf> {
    require_digest(digest)?;
    let name = digest
        .strip_prefix("sha256:")
        .context("content object key is not SHA-256")?;
    Ok(directory.join(format!("{name}.json")))
}

fn validate_content_object_exists(directory: &Path, digest: &str, label: &str) -> Result<PathBuf> {
    let path = content_object_path(directory, digest)?;
    let metadata = std::fs::symlink_metadata(&path)
        .with_context(|| format!("{label} is missing from the content-addressed store"))?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        bail!("{label} is not a regular non-symlink file");
    }
    if path
        .parent()
        .and_then(|parent| parent.canonicalize().ok())
        .as_deref()
        != Some(directory)
    {
        bail!("{label} escaped its content-addressed directory");
    }
    Ok(path)
}

fn read_canonical_content_object_with_len<T>(
    directory: &Path,
    digest: &str,
    label: &str,
) -> Result<(T, u64)>
where
    T: DeserializeOwned + Serialize,
{
    let path = validate_content_object_exists(directory, digest, label)?;
    let bytes = read_immutable_regular(&path)?;
    let object: T = serde_json::from_slice(&bytes)
        .with_context(|| format!("{label} is invalid or truncated JSON"))?;
    if canonical_bytes(&object)? != bytes {
        bail!("{label} is not exact canonical JSON");
    }
    Ok((object, bytes.len() as u64))
}

fn read_canonical_content_object<T>(directory: &Path, digest: &str, label: &str) -> Result<T>
where
    T: DeserializeOwned + Serialize,
{
    read_canonical_content_object_with_len(directory, digest, label).map(|(object, _)| object)
}

fn temporary_object_path(directory: &Path, label: &str, digest: &str) -> PathBuf {
    let counter = TEMPORARY_COUNTER.fetch_add(1, Ordering::Relaxed);
    directory.join(format!(
        ".{label}.{}.{}.{}.tmp",
        std::process::id(),
        counter,
        digest.strip_prefix("sha256:").unwrap_or("invalid")
    ))
}

fn publish_immutable_content_object<T>(
    directory: &Path,
    digest: &str,
    object: &T,
    label: &str,
) -> Result<u64>
where
    T: Serialize,
{
    require_digest(digest)?;
    let bytes = canonical_bytes(object)?;
    let target = content_object_path(directory, digest)?;
    let temporary = temporary_object_path(directory, "immutable", digest);
    let mut file = create_private_new(&temporary)?;
    let result = (|| -> Result<()> {
        file.write_all(&bytes)?;
        file.sync_all()?;
        match std::fs::hard_link(&temporary, &target) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == ErrorKind::AlreadyExists => {
                let existing = read_immutable_regular(&target)?;
                if existing != bytes {
                    bail!("immutable {label} identity collision");
                }
                Ok(())
            }
            Err(error) => Err(error.into()),
        }
    })();
    drop(file);
    let _ = std::fs::remove_file(&temporary);
    result?;
    sync_directory(directory)?;
    if read_immutable_regular(&target)? != bytes {
        bail!("published immutable {label} differs from canonical object");
    }
    Ok(bytes.len() as u64)
}

fn replace_canonical_catalog(directory: &Path, target: &Path, bytes: &[u8]) -> Result<()> {
    let digest = hash_bytes(bytes);
    let temporary = temporary_object_path(directory, "catalog", &digest);
    let mut file = create_private_new(&temporary)?;
    let result = (|| -> Result<()> {
        file.write_all(bytes)?;
        file.sync_all()?;
        std::fs::rename(&temporary, target)?;
        sync_directory(directory)
    })();
    drop(file);
    let _ = std::fs::remove_file(&temporary);
    result?;
    if read_immutable_regular(target)? != bytes {
        bail!("agent cache catalog differs after atomic replacement");
    }
    Ok(())
}

fn sync_directory(directory: &Path) -> Result<()> {
    File::open(directory)?.sync_all()?;
    Ok(())
}

fn create_checked_directory(path: &Path) -> Result<()> {
    match std::fs::create_dir(path) {
        Ok(()) => {}
        Err(error) if error.kind() == ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error.into()),
    }
    let metadata = std::fs::symlink_metadata(path)?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        bail!("K1 cache component is not a regular non-symlink directory");
    }
    Ok(())
}

fn create_private_checked_directory(path: &Path) -> Result<()> {
    let mut builder = std::fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    match builder.create(path) {
        Ok(()) => {}
        Err(error) if error.kind() == ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error.into()),
    }
    let metadata = std::fs::symlink_metadata(path)?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        bail!("compiler index root is not a regular non-symlink directory");
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o077 != 0 {
            bail!("compiler index root is not private");
        }
    }
    Ok(())
}

fn create_private_new(path: &Path) -> Result<File> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    Ok(options.open(path)?)
}

pub fn retain_attempt(
    path: &Path,
    repository: Option<&Path>,
    attempt: &KotlinAttempt,
) -> Result<()> {
    attempt.verify()?;
    validate_attempt_destination(path, repository)?;
    let mut bytes = canonical_bytes(attempt)?;
    bytes.push(b'\n');
    let parent = path.parent().context("attempt output has no parent")?;
    let temporary = parent.join(format!(
        ".codeclew-k1-attempt-{}-{}.tmp",
        std::process::id(),
        attempt
            .attempt_digest
            .strip_prefix("sha256:")
            .unwrap_or("invalid")
    ));
    let mut file = create_private_new(&temporary)?;
    let publication = (|| -> Result<()> {
        file.write_all(&bytes)?;
        file.sync_all()?;
        std::fs::hard_link(&temporary, path)
            .context("attempt output already exists or cannot be atomically retained")
    })();
    drop(file);
    let _ = std::fs::remove_file(&temporary);
    publication?;
    if read_immutable_regular(path)? != bytes {
        bail!("retained attempt differs after atomic publication");
    }
    Ok(())
}

pub fn validate_attempt_destination(path: &Path, repository: Option<&Path>) -> Result<()> {
    if !path.is_absolute() {
        bail!("attempt output path must be absolute");
    }
    let parent = path.parent().context("attempt output has no parent")?;
    let metadata = std::fs::symlink_metadata(parent)?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        bail!("attempt output parent must be a regular non-symlink directory");
    }
    require_private_directory(&metadata, "attempt output parent")?;
    let canonical_parent = parent.canonicalize()?;
    if canonical_parent != parent {
        bail!("attempt output parent must be canonical and have no symlinked ancestor");
    }
    if let Some(repository) = repository {
        let repository = repository.canonicalize()?;
        if canonical_parent.starts_with(&repository) || repository.starts_with(&canonical_parent) {
            bail!("attempt output must be outside and must not contain the source checkout");
        }
    }
    if std::fs::symlink_metadata(path).is_ok() {
        bail!("attempt output already exists; attempts are immutable");
    }
    Ok(())
}

fn require_digest(value: &str) -> Result<()> {
    if value.len() != 71
        || !value.starts_with("sha256:")
        || !value[7..]
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        bail!("malformed SHA-256 digest");
    }
    Ok(())
}

fn require_git_object(value: &str) -> Result<()> {
    if !matches!(value.len(), 40 | 64)
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        bail!("malformed Git object identity");
    }
    Ok(())
}

fn require_private_directory(metadata: &std::fs::Metadata, label: &str) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o022 != 0 {
            bail!("{label} must not be group- or world-writable");
        }
    }
    Ok(())
}

fn read_immutable_regular(path: &Path) -> Result<Vec<u8>> {
    let before = std::fs::symlink_metadata(path)?;
    if !before.is_file() || before.file_type().is_symlink() {
        bail!("immutable object is not a regular non-symlink file");
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        if before.nlink() != 1 || before.permissions().mode() & 0o022 != 0 {
            bail!("immutable object is shared-linked or writable by another principal");
        }
    }
    let mut file = File::open(path)?;
    let opened = file.metadata()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if opened.dev() != before.dev() || opened.ino() != before.ino() {
            bail!("immutable object identity changed while opening");
        }
    }
    let mut bytes = Vec::with_capacity(opened.len() as usize);
    file.read_to_end(&mut bytes)?;
    let after = std::fs::symlink_metadata(path)?;
    if !after.is_file() || after.file_type().is_symlink() || after.len() != opened.len() {
        bail!("immutable object type or size changed while reading");
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if after.dev() != opened.dev()
            || after.ino() != opened.ino()
            || after.len() != opened.len()
            || after.nlink() != 1
        {
            bail!("immutable object identity changed while reading");
        }
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use evidence_adapters::{AdapterIdentity, CostRecord, SnapshotInput};
    use tempfile::tempdir;

    fn manifest() -> Value {
        json!({
            "schema":"manifest/1",
            "analyzerCompilerVersion":"2.4.10",
            "orderedCompileClasspath":[],
            "buildState":{"seedDigest":hash_bytes(b"seed")},
        })
    }

    fn distribution() -> Value {
        json!({
            "treeHash":hash_bytes(b"worker-tree"),
            "buildInputDigest":hash_bytes(b"worker-inputs"),
            "pluginFingerprint":hash_bytes(b"worker-plugin"),
        })
    }

    fn output() -> AdapterOutput {
        let manifest = manifest();
        let distribution = distribution();
        let tree = hash_bytes(b"tree");
        let configuration = hash_bytes(b"configuration");
        let toolchain = hash_bytes(b"toolchain");
        let target = hash_bytes(b"target");
        let boundary_id = hash_bytes(b"boundary");
        let boundary = json!({
            "boundaryId":boundary_id,
            "kindUri":"codeclew.boundary/kotlin/test/1",
            "consequence":"ENUMERATION_INCOMPLETE",
            "origin":Value::Null,
        });
        let mut output = AdapterOutput {
            schema: evidence_adapters::ADAPTER_OUTPUT_SCHEMA.to_owned(),
            adapter: AdapterIdentity {
                adapter_id: "codeclew.kotlin-k2".to_owned(),
                version: "0.1.0".to_owned(),
                binary_digest: hash_bytes(b"adapter"),
                language_id: "kotlin".to_owned(),
            },
            snapshot_input: SnapshotInput {
                repository_tree_digest: tree.clone(),
                vcs_revision: Some("revision".to_owned()),
                dirty: false,
                sources: vec![evidence_adapters::SourceInput {
                    artifact_id: "source:src/Main.kt".to_owned(),
                    normalized_path: "src/Main.kt".to_owned(),
                    content_digest: hash_bytes(b"source"),
                    size_bytes: 6,
                    origin: "USER".to_owned(),
                }],
                build_system_uri: "codeclew.build-system/gradle/1".to_owned(),
                build_model_digest: hash_bytes(b"model"),
                build_configuration_digest: configuration.clone(),
                dependency_graph_digest: hash_bytes(b"dependencies"),
                toolchain: json!({
                    "toolUri":"codeclew.toolchain/kotlin-k2/1",
                    "version":"2.4.10",
                    "distributionDigest":toolchain,
                }),
                targets: vec![json!({
                    "targetId":":/main",
                    "configurationDigest":target,
                    "enabledFeatures":[],
                    "platform":"JVM",
                    "compilerFlags":[],
                })],
                relevant_environment: vec![],
                generated_sources_manifest_digest: hash_bytes(b"generated"),
            },
            capability_descriptors: vec![json!({
                "operationUri":"codeclew.relation/calls/1",
                "operationVersion":"1",
                "operationSpecificationDigest":hash_bytes(b"codeclew.relation/calls/1"),
                "languageId":"kotlin",
                "adapterId":"codeclew.kotlin-k2",
                "adapterVersion":"0.1.0",
                "toolchainDigest":toolchain,
                "buildConfigurationDigest":configuration,
                "targetDigest":target,
                "grade":"COMPILER_RESOLVED",
                "support":"SUPPORTED",
                "guaranteedEnumeration":"PARTIAL",
                "approximation":"SOUND_UNDER",
                "knownBoundaryKinds":["codeclew.boundary/kotlin/test/1"],
                "costClass":"codeclew.cost/compiler-frontend/1",
            })],
            entities: vec![],
            occurrences: vec![],
            facts: vec![],
            boundaries: vec![boundary.clone()],
            compiler_receipt: json!({
                "schema":"codeclew.compiler-receipt/0.1",
                "method":"K2_FIR_ANALYSIS",
                "status":"ACCEPTED",
                "grade":"COMPILER_CHECKED",
                "snapshotTreeDigest":tree,
                "claim":"test K2 result accepted",
                "providerPayload":{
                    "k2Validated":true,
                    "adapterBinaryDigest":hash_bytes(b"adapter"),
                    "analyzerCompilerVersion":"2.4.10",
                    "semanticInputManifestHash":canonical_hash(&manifest).unwrap(),
                    "semanticInputManifest":manifest,
                    "trustedWorkerDistribution":distribution,
                },
            }),
            impact: json!({
                "schema":"codeclew.impact-result/0.1",
                "status":"PARTIAL_BOUNDARY",
                "reason":"TEST_BOUNDARY",
                "closureSpecification":evidence_adapters::IMPACT_CLOSURE_SPEC,
                "affected":[],
                "paths":[],
                "boundaries":[boundary],
                "mandatoryObligations":[{
                    "id":"validate-test-boundary",
                    "kind":"codeclew.obligation/validate-boundary/1",
                    "mandatory":true,
                    "status":"UNKNOWN",
                    "reason":"TEST_BOUNDARY",
                }],
            }),
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
        output
    }

    fn key(output: &AdapterOutput) -> CacheKey {
        let manifest = manifest();
        CacheKey::exact(
            &output.snapshot_input.repository_tree_digest,
            output.snapshot_input.vcs_revision.as_deref(),
            output.snapshot_input.dirty,
            ":/main",
            &output.adapter.binary_digest,
            &output.adapter.version,
            distribution(),
            json!({"compilerVersion":"2.4.10","workerVersion":"0.1.0","protocol":"1.0"}),
            &manifest,
            &canonical_hash(&manifest).unwrap(),
        )
        .unwrap()
    }

    fn core(output: &AdapterOutput) -> CoreBindingSummary {
        CoreBindingSummary {
            schema: "codeclew.evidence-core-binding/0.1".to_owned(),
            snapshot_id: output.snapshot_input.repository_tree_digest.clone(),
            bundle_digest: hash_bytes(b"bundle"),
            capability_count: 0,
            batch_count: 0,
            fact_count: 0,
            obligation_graph_count: 0,
            impact_receipt_count: 0,
            canonical_bundle_bytes: 0,
            translation_micros: 0,
            validation_micros: 0,
            digest_micros: 0,
            total_micros: 0,
        }
    }

    fn graph_output() -> AdapterOutput {
        let mut output = output();
        let mut fact_ids = [hash_bytes(b"fact-a"), hash_bytes(b"fact-b")];
        fact_ids.sort();
        output.entities = vec![
            json!({
                "opaqueId":"entity:caller",
                "resolution":"RESOLVED",
                "primaryDefinition":{"artifactId":"source:src/Main.kt","startByte":0,"endByte":1},
                "languagePayload":{"compilerCallableId":"example.caller()"},
            }),
            json!({
                "opaqueId":"entity:target",
                "resolution":"RESOLVED",
                "primaryDefinition":{"artifactId":"source:src/Main.kt","startByte":2,"endByte":3},
                "languagePayload":{"compilerCallableId":"example.target()"},
            }),
            json!({
                "opaqueId":"entity:second-caller",
                "resolution":"RESOLVED",
                "primaryDefinition":{"artifactId":"source:src/Main.kt","startByte":4,"endByte":5},
                "languagePayload":{"compilerCallableId":"example.secondCaller()"},
            }),
        ];
        output.facts = vec![
            json!({
                "factId":fact_ids[1],
                "owner":"entity:caller",
                "target":"entity:target",
                "truth":"TRUE",
                "relation":"codeclew.relation/calls/1",
            }),
            json!({
                "factId":fact_ids[0],
                "owner":"entity:second-caller",
                "target":"entity:target",
                "truth":"TRUE",
                "relation":"codeclew.relation/calls/1",
            }),
        ];
        output.boundaries.push(json!({
            "boundaryId":hash_bytes(b"target-boundary"),
            "kindUri":"codeclew.boundary/kotlin/query-specific/1",
            "consequence":"ENUMERATION_INCOMPLETE",
            "applicability":{
                "schema":"codeclew.boundary-applicability/0.1",
                "effect":"TOPOLOGY_ENUMERATION",
                "operations":["codeclew.relation/calls/1"],
                "direction":"INCOMING",
                "locality":"EXACT",
                "ownerQueryKeys":["entity:caller"],
                "targetQueryKeys":["entity:target"],
            },
        }));
        output.boundaries.push(json!({
            "boundaryId":hash_bytes(b"legacy-details-boundary"),
            "kindUri":"codeclew.boundary/kotlin/legacy-details/1",
            "consequence":"ENUMERATION_INCOMPLETE",
            "details":{"queryKeys":["entity:target"]},
        }));
        output.boundaries.push(json!({
            "boundaryId":hash_bytes(b"missing-effect-boundary"),
            "kindUri":"codeclew.boundary/kotlin/missing-effect/1",
            "consequence":"ENUMERATION_INCOMPLETE",
            "applicability":{
                "schema":"codeclew.boundary-applicability/0.1",
                "operations":["codeclew.relation/calls/1"],
                "direction":"INCOMING",
                "locality":"EXACT",
                "ownerQueryKeys":["entity:caller"],
                "targetQueryKeys":["entity:target"],
            },
        }));
        output.seal().unwrap();
        output
    }

    fn cache_for(repository: &tempfile::TempDir, state: &tempfile::TempDir) -> SemanticCache {
        SemanticCache::open(&state.path().canonicalize().unwrap(), repository.path()).unwrap()
    }

    fn raw_object(output: &AdapterOutput) -> (CacheKey, SemanticCacheObject) {
        let key = key(output);
        let core = validate_kotlin_core_binding(output).unwrap();
        let object = SemanticCacheObject::new(&key, output, &core).unwrap();
        (key, object)
    }

    fn publish_raw_object(cache: &SemanticCache, object: &SemanticCacheObject) {
        publish_immutable_content_object(
            &cache.objects,
            &object.cache_key,
            object,
            "semantic cache object",
        )
        .unwrap();
    }

    #[test]
    fn semantic_cache_creates_an_explicit_missing_private_state_root() {
        let repository = tempfile::tempdir().unwrap();
        let parent = tempfile::tempdir().unwrap();
        let state_root = parent.path().canonicalize().unwrap().join("state");
        assert!(!state_root.exists());

        let cache = SemanticCache::open(&state_root, repository.path()).unwrap();

        assert_eq!(cache.canonical_root(), state_root.as_path());
        let metadata = std::fs::symlink_metadata(&state_root).unwrap();
        assert!(metadata.is_dir());
        assert!(!metadata.file_type().is_symlink());
        let compiler_index = cache.compiler_index_root();
        assert_eq!(compiler_index, state_root.join("compiler-index"));
        let compiler_metadata = std::fs::symlink_metadata(compiler_index).unwrap();
        assert!(compiler_metadata.is_dir());
        assert!(!compiler_metadata.file_type().is_symlink());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(metadata.permissions().mode() & 0o777, 0o700);
            assert_eq!(compiler_metadata.permissions().mode() & 0o777, 0o700);
        }
    }

    #[cfg(unix)]
    #[test]
    fn semantic_cache_rejects_a_public_compiler_index_root() {
        use std::os::unix::fs::PermissionsExt;

        let repository = tempfile::tempdir().unwrap();
        let state = tempfile::tempdir().unwrap();
        std::fs::set_permissions(state.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let compiler_index = state.path().join("compiler-index");
        std::fs::create_dir(&compiler_index).unwrap();
        std::fs::set_permissions(&compiler_index, std::fs::Permissions::from_mode(0o755)).unwrap();

        assert!(SemanticCache::open(state.path(), repository.path()).is_err());
    }

    fn prepared_refusal() -> PreparedRefusal {
        let mut refusal = PreparedRefusal {
            schema: PREPARED_REFUSAL_SCHEMA.to_owned(),
            series_id: K1_SERIES_ID.to_owned(),
            cohort: "QUALIFICATION".to_owned(),
            entry: "K1-Q02".to_owned(),
            commit: "0123456789abcdef0123456789abcdef01234567".to_owned(),
            git_tree: "89abcdef0123456789abcdef0123456789abcdef".to_owned(),
            selected_compilation: ":app/main".to_owned(),
            build_dsl: "GRADLE_KOTLIN_DSL".to_owned(),
            failure_stage: "DEPENDENCY_PREPARATION".to_owned(),
            reason_code: "OFFLINE_MODEL_PROBE_FAILED".to_owned(),
            safe_detail_digest: hash_bytes(b"strict offline Gradle FileLock denial"),
            cost: PreparedRefusalCost {
                wall_micros: 1,
                stdout_bytes: 0,
                stderr_bytes: 1,
                exit_code: 1,
            },
            sandbox_profile_sha256: hash_bytes(b"strict offline sandbox profile"),
            source_tree_sha256: hash_bytes(b"source tree"),
            candidate_tools_sha256: hash_bytes(b"candidate tools"),
            build_input_digest: hash_bytes(b"build inputs"),
            preparation_receipt_digest: hash_bytes(b"preparation receipt"),
            object_digest: String::new(),
        };
        refusal.object_digest = canonical_hash(&refusal).unwrap();
        refusal
    }

    #[test]
    fn prepared_refusal_rejects_k1_11_predecessor_identity() {
        let refusal = prepared_refusal();
        refusal.verify().unwrap();

        let mut predecessor_series = refusal.clone();
        predecessor_series.series_id = "KOTLIN_REAL_REPOSITORY_K1_11_2026_08_13".to_owned();
        assert!(predecessor_series.verify().is_err());

        let mut predecessor_schema = refusal;
        predecessor_schema.schema =
            "codeclew.kotlin-k1-dependency-preparation-refusal/0.10".to_owned();
        assert!(predecessor_schema.verify().is_err());
    }

    #[test]
    fn attempt_digest_detects_mutation() {
        let mut attempt = KotlinAttempt::terminal(
            "FAILED",
            "WORKER_START",
            "WORKER_CRASHED",
            "private detail",
            json!({}),
            json!({}),
            json!({}),
            vec![],
            json!({"status":"DISABLED"}),
            AttemptTelemetry::new(),
        )
        .unwrap();
        attempt.verify().unwrap();
        assert_ne!(attempt.detail_digest.as_deref(), Some("private detail"));
        attempt.reason_code = Some("FORGED".to_owned());
        assert!(attempt.verify().is_err());
    }

    #[test]
    fn attempt_rejects_status_field_and_telemetry_forgery_even_if_resealed() {
        let mut attempt = KotlinAttempt::terminal(
            "FAILED",
            "WORKER_START",
            "WORKER_CRASHED",
            "detail",
            json!({}),
            json!({}),
            json!({}),
            vec![],
            json!({"status":"DISABLED"}),
            AttemptTelemetry::new(),
        )
        .unwrap();
        attempt.status = "SUCCEEDED".to_owned();
        attempt.seal().unwrap();
        assert!(attempt.verify().is_err());

        attempt.status = "FAILED".to_owned();
        attempt.cost.model_calls = 1;
        attempt.seal().unwrap();
        assert!(attempt.verify().is_err());

        attempt.cost.model_calls = 0;
        attempt.cost.cache_requests = 0;
        attempt.cost.cache_hits = 1;
        attempt.seal().unwrap();
        assert!(attempt.verify().is_err());
    }

    #[test]
    fn successful_attempt_requires_explicit_cache_hit_or_publication_authority() {
        let output = output();
        let mut telemetry = AttemptTelemetry::new();
        telemetry.cache_requests = 1;
        telemetry.cache_bytes_written = 123;
        let mut attempt = KotlinAttempt::success(
            &output,
            core(&output),
            json!({"runPhase":"COLD"}),
            json!({}),
            json!({
                "status":"PUBLISHED_COLD",
                "hit":false,
                "keyDigest":hash_bytes(b"key"),
                "semanticOutputDigest":semantic_output_digest(&output).unwrap(),
                "semanticFactsDigest":semantic_facts_digest(&output).unwrap(),
            }),
            telemetry,
        )
        .unwrap();
        attempt.verify().unwrap();

        attempt.cache["hit"] = Value::Bool(true);
        attempt.seal().unwrap();
        assert!(attempt.verify().is_err());
    }

    #[test]
    fn terminal_identity_ignores_absolute_staging_path() {
        let make = |path: &str, millis: u64| {
            KotlinAttempt::terminal(
                "FAILED",
                "BUILD_DISCOVERY",
                "BUILD_DISCOVERY_FAILED",
                &format!("provider failed at {path}/temporary/output after {millis} ms"),
                json!({}),
                json!({}),
                json!({}),
                vec![],
                json!({}),
                AttemptTelemetry::new(),
            )
            .unwrap()
        };
        let first = make("/private/a", 19);
        let second = make("/private/b", 4200);
        assert_eq!(first.detail_digest, second.detail_digest);
        assert_eq!(
            first.terminal_semantic_digest,
            second.terminal_semantic_digest
        );
    }

    #[test]
    fn cache_key_binds_ordered_manifest() {
        let output = output();
        let first = key(&output);
        let mut changed = first.inputs["semanticInputManifest"].clone();
        changed["orderedCompileClasspath"] = json!(["b", "a"]);
        let second = CacheKey::exact(
            &output.snapshot_input.repository_tree_digest,
            output.snapshot_input.vcs_revision.as_deref(),
            false,
            ":/main",
            &output.adapter.binary_digest,
            &output.adapter.version,
            first.inputs["workerDistribution"].clone(),
            first.inputs["workerCapabilities"].clone(),
            &changed,
            &canonical_hash(&changed).unwrap(),
        )
        .unwrap();
        assert_ne!(first.digest, second.digest);
        assert!(first.inputs.get("query").is_none());
    }

    #[test]
    fn cache_payload_receipt_must_bind_exact_key_inputs() {
        let mut output = output();
        let key = key(&output);
        ensure_cache_payload_matches_key(&output, &key).unwrap();
        output.compiler_receipt["providerPayload"]["semanticInputManifestHash"] =
            Value::String(hash_bytes(b"forged manifest"));
        output.seal().unwrap();
        assert!(ensure_cache_payload_matches_key(&output, &key).is_err());
    }

    #[test]
    fn accepted_compiler_receipt_without_k2_validation_is_rejected() {
        let mut output = output();
        validate_kotlin_core_binding(&output).unwrap();
        output.compiler_receipt["providerPayload"]["k2Validated"] = Value::Bool(false);
        output.seal().unwrap();
        assert!(validate_kotlin_core_binding(&output).is_err());
    }

    #[test]
    fn cache_publishes_and_revalidates_exact_core_bound_object() {
        let repository = tempdir().unwrap();
        let state = tempdir().unwrap();
        let state_root = state.path().canonicalize().unwrap();
        let cache = SemanticCache::open(&state_root, repository.path()).unwrap();
        let output = output();
        let key = key(&output);
        let core = validate_kotlin_core_binding(&output).unwrap();

        let publication = cache.publish(&key, &output, &core).unwrap();
        assert!(publication.bytes_written > 0);
        let CacheLookup::Hit(hit) = cache.lookup(&key).unwrap() else {
            panic!("published exact cache object must revalidate as a hit");
        };
        assert_eq!(hit.output.output_digest, output.output_digest);
        assert_eq!(
            semantic_facts_digest(&hit.output).unwrap(),
            semantic_facts_digest(&output).unwrap()
        );
    }

    #[test]
    fn bounded_agent_index_answers_without_loading_full_adapter_output() {
        let repository = tempdir().unwrap();
        let state = tempdir().unwrap();
        let cache = cache_for(&repository, &state);
        let output = graph_output();
        let key = key(&output);
        let object = SemanticCacheObject::new(&key, &output, &core(&output)).unwrap();
        publish_raw_object(&cache, &object);
        let (manifest, _) = cache.publish_agent_graph(&object).unwrap();
        cache.publish_catalog(&object, &manifest).unwrap();

        let lookup = cache
            .lookup_agent_graph_query(
                &output.snapshot_input.repository_tree_digest,
                output.snapshot_input.vcs_revision.as_deref(),
                false,
                ":/main",
                &output.adapter.adapter_id,
                &output.adapter.version,
                Some(&hash_bytes(b"seed")),
            )
            .unwrap()
            .unwrap();
        assert_eq!(lookup.index_telemetry.legacy_objects_scanned, 0);
        assert_eq!(
            lookup
                .index
                .projection_authority()
                .pointer("/snapshot/selectedCompilation")
                .and_then(Value::as_str),
            Some(":/main")
        );
        assert_eq!(
            lookup
                .index
                .resolve_seed("example.target()")
                .unwrap()
                .entity_id,
            "entity:target"
        );
        let incoming = lookup.index.incoming_facts("entity:target").unwrap();
        assert_eq!(incoming.len(), 2);
        assert_eq!(
            incoming
                .iter()
                .map(|fact| fact.get("factId").and_then(Value::as_str).unwrap())
                .collect::<Vec<_>>(),
            output
                .facts
                .iter()
                .map(|fact| fact.get("factId").and_then(Value::as_str).unwrap())
                .collect::<Vec<_>>()
        );
        assert!(lookup.index.entity("entity:caller").unwrap().is_some());
        assert_eq!(
            lookup
                .index
                .boundaries_for_query_key("entity:target")
                .unwrap()
                .len(),
            4
        );
        assert_eq!(
            lookup
                .index
                .boundaries_for_query_key("entity:caller")
                .unwrap()
                .len(),
            3
        );
        assert_eq!(lookup.index.project_boundary_summary()["count"], 4);
        assert_eq!(lookup.index.project_boundary_summary()["globalCount"], 3);
    }

    #[test]
    fn agent_cache_selector_and_catalog_reject_prior_normalization_contract() {
        let selector = AgentCacheSelector::exact(
            &hash_bytes(b"tree"),
            Some("revision"),
            false,
            ":/main",
            "codeclew.kotlin-k2",
            "0.1.0",
            Some(&hash_bytes(b"seed")),
        )
        .unwrap();
        let mut prior_selector = selector.clone();
        prior_selector.normalization_contract_digest = canonical_hash(&json!({
            "contract":"codeclew.kotlin-agent-graph-normalization/0.4",
            "adapterId":selector.adapter_id.as_str(),
            "adapterVersion":selector.adapter_version.as_str(),
            "projection":"BOUNDED_REVERSE_RESOLVED_RELATIONS",
        }))
        .unwrap();

        assert_ne!(
            canonical_hash(&prior_selector).unwrap(),
            selector.digest().unwrap()
        );
        assert!(prior_selector.verify().is_err());

        let mut prior_catalog = AgentCacheCatalog {
            schema: AGENT_CACHE_CATALOG_SCHEMA.to_owned(),
            selector_digest: canonical_hash(&prior_selector).unwrap(),
            normalization_contract_digest: prior_selector.normalization_contract_digest.clone(),
            selector: prior_selector,
            semantic_cache_key: hash_bytes(b"semantic-cache-key"),
            semantic_object_digest: hash_bytes(b"semantic-object"),
            agent_graph_manifest_digest: hash_bytes(b"agent-graph-manifest"),
            record_digest: String::new(),
        };
        prior_catalog.record_digest = canonical_hash(&prior_catalog).unwrap();
        assert!(prior_catalog.verify(&selector).is_err());
    }

    #[test]
    fn agent_catalog_miss_never_scans_legacy_semantic_objects() {
        let repository = tempdir().unwrap();
        let state = tempdir().unwrap();
        let cache = cache_for(&repository, &state);
        let output = output();
        let (_, object) = raw_object(&output);
        publish_raw_object(&cache, &object);
        let corrupt_unrelated = cache
            .objects
            .join(format!("{}.json", &hash_bytes(b"unrelated-corrupt")[7..]));
        std::fs::write(corrupt_unrelated, b"{").unwrap();

        let lookup = cache
            .lookup_agent_graph_query(
                &output.snapshot_input.repository_tree_digest,
                output.snapshot_input.vcs_revision.as_deref(),
                false,
                ":/main",
                &output.adapter.adapter_id,
                &output.adapter.version,
                Some(&hash_bytes(b"seed")),
            )
            .unwrap();

        assert!(lookup.is_none());
        assert_eq!(std::fs::read_dir(&cache.agent_catalogs).unwrap().count(), 0);
    }

    #[test]
    fn agent_graph_rejects_duplicate_missing_corrupt_and_symlinked_shards() {
        let repository = tempdir().unwrap();
        let state = tempdir().unwrap();
        let cache = cache_for(&repository, &state);
        let output = graph_output();
        let key = key(&output);
        let object = SemanticCacheObject::new(&key, &output, &core(&output)).unwrap();
        let (manifest, shards) = build_agent_graph_index(&object).unwrap();
        let (published, _) = cache.publish_agent_graph(&object).unwrap();
        let first_ref = published.shards.first().unwrap();

        let mut duplicate = manifest.clone();
        duplicate.shards.push(first_ref.clone());
        duplicate.seal().unwrap();
        publish_immutable_content_object(
            &cache.agent_graph_manifests,
            &duplicate.object_digest,
            &duplicate,
            "agent graph manifest",
        )
        .unwrap();
        assert!(
            cache
                .load_agent_graph_manifest(&duplicate.object_digest, None)
                .is_err()
        );

        let shard_path =
            content_object_path(&cache.agent_graph_shards, &first_ref.object_digest).unwrap();
        std::fs::remove_file(&shard_path).unwrap();
        assert!(
            cache
                .load_agent_graph_manifest(&published.object_digest, None)
                .is_err()
        );
        let shard = shards
            .iter()
            .find(|shard| shard.object_digest == first_ref.object_digest)
            .unwrap();
        publish_immutable_content_object(
            &cache.agent_graph_shards,
            &shard.object_digest,
            shard,
            "agent graph shard",
        )
        .unwrap();
        std::fs::write(&shard_path, b"{}").unwrap();
        let (index, _) = cache
            .load_agent_graph_manifest(&published.object_digest, None)
            .unwrap();
        let query_key = shard
            .entities
            .keys()
            .chain(shard.aliases.keys())
            .chain(shard.incoming.keys())
            .chain(shard.keyed_boundaries.keys())
            .next()
            .unwrap();
        assert!(index.load_bucket(query_key).is_err());

        #[cfg(unix)]
        {
            std::fs::remove_file(&shard_path).unwrap();
            std::os::unix::fs::symlink(
                content_object_path(&cache.agent_graph_manifests, &published.object_digest)
                    .unwrap(),
                &shard_path,
            )
            .unwrap();
            assert!(
                cache
                    .load_agent_graph_manifest(&published.object_digest, None)
                    .is_err()
            );
        }
    }

    #[test]
    fn cold_and_warm_semantic_digests_ignore_measurements_but_not_projection() {
        let mut cold = output();
        cold.impact = json!({
            "schema":"codeclew.impact-result/0.1",
            "status":"COMPLETE_IN_SCOPE",
            "seedEntity":"a",
            "affected":[{"entityId":"a","impactClass":"DEFINITE","depth":0}],
            "paths":[],
            "mandatoryObligations":[],
            "boundaries":[],
            "providerPayload":{
                "proposedSeedEntity":"a",
                "selectionAuthority":"DETERMINISTIC_LEXICOGRAPHIC_CANDIDATE",
            },
            "queryMicros":11,
        });
        cold.cost.cold_index_micros = 101;
        cold.cost.cache_requests = 1;
        cold.seal().unwrap();

        let mut warm = cold.clone();
        warm.impact["queryMicros"] = Value::from(37);
        warm.cost.cold_index_micros = 0;
        warm.cost.warm_index_micros = 19;
        warm.cost.cache_hits = 1;
        warm.seal().unwrap();

        assert_ne!(cold.output_digest, warm.output_digest);
        assert_eq!(
            semantic_output_digest(&cold).unwrap(),
            semantic_output_digest(&warm).unwrap()
        );
        assert_eq!(
            semantic_facts_digest(&cold).unwrap(),
            semantic_facts_digest(&warm).unwrap()
        );

        warm.impact["providerPayload"]["selectionAuthority"] = Value::String("FORGED".to_owned());
        warm.seal().unwrap();
        assert_ne!(
            semantic_output_digest(&cold).unwrap(),
            semantic_output_digest(&warm).unwrap()
        );
    }

    #[test]
    fn cache_rejects_corruption_and_symlink() {
        let repository = tempdir().unwrap();
        let state = tempdir().unwrap();
        let state_root = state.path().canonicalize().unwrap();
        let cache = SemanticCache::open(&state_root, repository.path()).unwrap();
        let output = output();
        // This synthetic envelope is not a complete evidence-core bundle, so
        // publish is deliberately tested with a structurally forged binding
        // only at the filesystem layer below.
        let key = key(&output);
        let path = cache.object_path(&key).unwrap();
        std::fs::write(&path, b"{\"truncated\":").unwrap();
        assert!(cache.lookup(&key).is_err());
        std::fs::remove_file(&path).unwrap();
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink("missing", &path).unwrap();
            assert!(cache.lookup(&key).is_err());
            std::fs::remove_file(&path).unwrap();
            let linked_source = state.path().join("linked-source.json");
            std::fs::write(&linked_source, b"{}").unwrap();
            std::fs::hard_link(&linked_source, &path).unwrap();
            assert!(cache.lookup(&key).is_err());
        }
    }

    #[test]
    fn attempt_retention_is_create_only_and_external() {
        let repository = tempdir().unwrap();
        let state = tempdir().unwrap();
        let state_root = state.path().canonicalize().unwrap();
        let repository_root = repository.path().canonicalize().unwrap();
        let attempt = KotlinAttempt::terminal(
            "REFUSED",
            "BUILD_DISCOVERY",
            "UNSUPPORTED_PROJECT_CONFIGURATION",
            "detail",
            json!({}),
            json!({}),
            json!({}),
            vec![],
            json!({}),
            AttemptTelemetry::new(),
        )
        .unwrap();
        let path = state_root.join("attempt.json");
        retain_attempt(&path, Some(repository.path()), &attempt).unwrap();
        assert!(retain_attempt(&path, Some(repository.path()), &attempt).is_err());
        assert!(
            retain_attempt(
                &repository_root.join("inside.json"),
                Some(repository.path()),
                &attempt
            )
            .is_err()
        );
    }
}
