use anyhow::{Context, Result, bail};
use clap::{Args, Parser, Subcommand};
use evidence_adapters::{
    ADAPTER_OUTPUT_SCHEMA, AdapterOutput, CoreBindingSummary, canonical_bytes, canonical_hash,
    hash_bytes, validate_core_binding,
};
use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

const PROJECTION_SCHEMA: &str = "codeclew.repository-impact-projection/0.1";
const REFUSAL_SCHEMA: &str = "codeclew.evidence-run-refusal/0.1";
const MAX_ADAPTER_OUTPUT_BYTES: usize = 64 * 1024 * 1024;

#[derive(Parser)]
#[command(
    name = "codeclew-evidence",
    about = "Language-neutral, read-only semantic evidence and impact projection"
)]
struct Cli {
    #[command(subcommand)]
    command: EvidenceCommand,
}

#[derive(Subcommand)]
enum EvidenceCommand {
    #[command(about = "Run an exact pinned language adapter and emit a bounded projection")]
    Run(RunArgs),
}

#[derive(Args)]
struct RunArgs {
    #[arg(long)]
    repo: PathBuf,
    #[arg(long)]
    adapter: PathBuf,
    #[arg(long)]
    adapter_sha256: String,
    /// Existing private directory used as the content-addressed evidence
    /// store. Objects are immutable and verified after publication.
    #[arg(long)]
    store: PathBuf,
    /// Opaque adapter-owned arguments. The shared runtime forwards them
    /// without interpreting language semantics. Use `--adapter-arg=value`
    /// for values beginning with a dash.
    #[arg(long, allow_hyphen_values = true)]
    adapter_arg: Vec<String>,
    #[arg(long)]
    seed_entity: Option<String>,
    #[arg(long, default_value_t = 2)]
    max_depth: usize,
    #[arg(long, default_value_t = 128)]
    max_entities: usize,
    #[arg(long, default_value_t = 32 * 1024)]
    max_projection_bytes: usize,
    #[arg(long, default_value_t = 1, value_parser = clap::value_parser!(u8).range(1..=2))]
    repetitions: u8,
}

fn main() {
    if let Err(error) = dispatch() {
        let detail = format!("{error:#}");
        let runtime = runtime_identity().ok();
        let refusal = json!({
            "schema":REFUSAL_SCHEMA,
            "status":"REFUSED",
            "reasonUri":"codeclew.refusal/evidence-run-failed/1",
            "failureStage":"ADAPTER_OR_EVIDENCE_VALIDATION",
            "detailDigest":hash_bytes(detail.as_bytes()),
            "runtime":runtime,
            "boundaries":[{
                "kindUri":"codeclew.boundary/provider-or-evidence-failure/1",
                "consequence":"PROOF_INVALID"
            }]
        });
        match canonical_bytes(&refusal) {
            Ok(bytes) => println!("{}", String::from_utf8_lossy(&bytes)),
            Err(_) => println!("{{\"schema\":\"{REFUSAL_SCHEMA}\",\"status\":\"REFUSED\"}}"),
        }
        std::process::exit(2);
    }
}

fn dispatch() -> Result<()> {
    match Cli::parse().command {
        EvidenceCommand::Run(args) => run(args),
    }
}

fn run(args: RunArgs) -> Result<()> {
    let orchestration_start = Instant::now();
    let repo = canonical_directory(&args.repo)?;
    let adapter = pinned_executable(&args.adapter, &args.adapter_sha256)?;
    let runtime = runtime_identity()?;
    let first_start = Instant::now();
    let first = invoke_adapter(&adapter, &repo, &args)?;
    let cold_wall_micros = first_start.elapsed().as_micros() as u64;
    let raw_validation_start = Instant::now();
    validate_output(&first, &args.adapter_sha256)?;
    let raw_validation_micros = raw_validation_start.elapsed().as_micros() as u64;
    let core_binding = validate_core_binding(&first)?;
    let store_start = Instant::now();
    let store = EvidenceStore::open(&args.store)?;
    let adapter_bytes = canonical_bytes(&first)?;
    let adapter_object_digest = hash_bytes(&adapter_bytes);
    let adapter_object = store.publish(&adapter_bytes, &adapter_object_digest)?;
    let store_write_micros = store_start.elapsed().as_micros() as u64;
    let store_read_start = Instant::now();
    let stored_adapter_bytes = store.read_verified(&adapter_object_digest)?;
    if stored_adapter_bytes != adapter_bytes {
        bail!("evidence store round-trip changed canonical adapter bytes");
    }
    let store_read_micros = store_read_start.elapsed().as_micros() as u64;

    let mut warm_wall_micros = None;
    let mut warm_adapter_cost = None;
    if args.repetitions == 2 {
        let warm_start = Instant::now();
        let warm = invoke_adapter(&adapter, &repo, &args)?;
        warm_wall_micros = Some(warm_start.elapsed().as_micros() as u64);
        validate_output(&warm, &args.adapter_sha256)?;
        if semantic_output_digest(&first)? != semantic_output_digest(&warm)? {
            bail!("warm adapter run changed semantic output for the exact snapshot");
        }
        warm_adapter_cost = Some(warm.cost);
    }

    let mut projection = build_projection(
        &first,
        &adapter,
        &runtime,
        &args,
        cold_wall_micros,
        raw_validation_micros,
        &adapter_object,
        store_write_micros,
        store_read_micros,
        warm_wall_micros,
        warm_adapter_cost.as_ref(),
        &core_binding,
    )?;
    projection["cost"]["orchestrationPreSerializationMicros"] =
        Value::from(orchestration_start.elapsed().as_micros() as u64);
    enforce_budget(&mut projection, args.max_projection_bytes)?;
    let digest = canonical_hash(&projection)?;
    projection["projectionDigest"] = Value::String(digest);
    let bytes = canonical_bytes(&projection)?;
    if bytes.len() > args.max_projection_bytes {
        bail!("bounded projection exceeds maxProjectionBytes after truncation");
    }
    let mut stdout = std::io::stdout().lock();
    stdout.write_all(&bytes)?;
    stdout.write_all(b"\n")?;
    Ok(())
}

fn invoke_adapter(adapter: &Path, repo: &Path, args: &RunArgs) -> Result<AdapterOutput> {
    let mut command = Command::new(adapter);
    command.args(&args.adapter_arg);
    command.arg("--repo").arg(repo);
    if let Some(seed) = &args.seed_entity {
        command.arg("--seed-entity").arg(seed);
    }
    command
        .arg("--max-depth")
        .arg(args.max_depth.to_string())
        .arg("--max-entities")
        .arg(args.max_entities.to_string());
    let output = command.output()?;
    if !output.status.success() {
        bail!(
            "adapter failed with {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        );
    }
    if output.stdout.len() > MAX_ADAPTER_OUTPUT_BYTES {
        bail!("adapter output exceeds the 64 MiB authority limit");
    }
    let parsed: AdapterOutput = serde_json::from_slice(&output.stdout)
        .context("adapter stdout is not the exact canonical adapter envelope")?;
    if canonical_bytes(&parsed)? != trim_one_newline(&output.stdout) {
        bail!("adapter stdout is not canonical JSON plus one optional newline");
    }
    Ok(parsed)
}

fn trim_one_newline(bytes: &[u8]) -> Vec<u8> {
    bytes.strip_suffix(b"\n").unwrap_or(bytes).to_vec()
}

fn validate_output(output: &AdapterOutput, expected_adapter_sha: &str) -> Result<()> {
    if output.schema != ADAPTER_OUTPUT_SCHEMA {
        bail!("unsupported adapter output schema {}", output.schema);
    }
    output.verify_seal()?;
    let expected = normalize_digest(expected_adapter_sha)?;
    if output.adapter.binary_digest != expected {
        bail!("adapter envelope binary digest differs from pinned executable");
    }
    require_digest(&output.snapshot_input.repository_tree_digest)?;
    require_digest(&output.snapshot_input.build_model_digest)?;
    require_digest(&output.snapshot_input.build_configuration_digest)?;
    require_digest(&output.snapshot_input.dependency_graph_digest)?;
    require_digest(&output.snapshot_input.generated_sources_manifest_digest)?;
    let mut prior = None;
    let mut source_ids = BTreeSet::new();
    for source in &output.snapshot_input.sources {
        if prior
            .as_ref()
            .is_some_and(|value: &String| value >= &source.artifact_id)
        {
            bail!("snapshot sources are not strictly ordered by artifactId");
        }
        prior = Some(source.artifact_id.clone());
        if !source_ids.insert(source.artifact_id.clone()) {
            bail!("snapshot contains duplicate source artifactId");
        }
        require_digest(&source.content_digest)?;
        if source.normalized_path.starts_with('/')
            || source
                .normalized_path
                .split('/')
                .any(|part| part.is_empty() || part == "." || part == "..")
        {
            bail!("source path is not normalized and contained");
        }
    }
    validate_capabilities(output)?;
    validate_facts_and_boundaries(output)?;
    validate_compiler_receipt(output)?;
    validate_impact(output)?;
    Ok(())
}

fn validate_capabilities(output: &AdapterOutput) -> Result<()> {
    let toolchain_digest = output
        .snapshot_input
        .toolchain
        .get("distributionDigest")
        .and_then(Value::as_str)
        .context("snapshot toolchain misses distributionDigest")?;
    require_digest(toolchain_digest)?;
    let target_digests = output
        .snapshot_input
        .targets
        .iter()
        .map(|target| {
            target
                .get("configurationDigest")
                .and_then(Value::as_str)
                .context("snapshot target misses configurationDigest")
        })
        .collect::<Result<BTreeSet<_>>>()?;
    if target_digests.is_empty() {
        bail!("snapshot must declare at least one target");
    }
    let mut operations = BTreeSet::new();
    for capability in &output.capability_descriptors {
        let object = capability
            .as_object()
            .context("capability descriptor must be an object")?;
        for field in [
            "operationUri",
            "languageId",
            "adapterId",
            "adapterVersion",
            "grade",
            "support",
            "guaranteedEnumeration",
            "operationVersion",
            "operationSpecificationDigest",
            "toolchainDigest",
            "buildConfigurationDigest",
            "targetDigest",
            "approximation",
            "costClass",
        ] {
            if !object.get(field).is_some_and(Value::is_string) {
                bail!("capability descriptor misses string {field}");
            }
        }
        if capability["languageId"] != output.adapter.language_id
            || capability["adapterId"] != output.adapter.adapter_id
            || capability["adapterVersion"] != output.adapter.version
        {
            bail!("capability tuple differs from adapter identity");
        }
        let operation = capability["operationUri"].as_str().unwrap();
        if !operations.insert(operation.to_owned()) {
            bail!("duplicate capability operation URI");
        }
        if capability["support"].as_str() != Some("SUPPORTED") {
            bail!("unsupported capabilities require a typed refusal, not a descriptor");
        }
        if !matches!(
            capability["grade"].as_str(),
            Some(
                "NAVIGATION"
                    | "COMPILER_RESOLVED"
                    | "COMPILER_CHECKED"
                    | "SOUND_STATIC_IN_SCOPE"
                    | "STATICALLY_APPROXIMATED"
                    | "TESTED"
                    | "RUNTIME_OBSERVED"
            )
        ) {
            bail!("capability has an unknown evidence grade");
        }
        require_digest(capability["operationSpecificationDigest"].as_str().unwrap())?;
        require_digest(capability["toolchainDigest"].as_str().unwrap())?;
        require_digest(capability["buildConfigurationDigest"].as_str().unwrap())?;
        require_digest(capability["targetDigest"].as_str().unwrap())?;
        if capability["toolchainDigest"].as_str() != Some(toolchain_digest)
            || capability["buildConfigurationDigest"].as_str()
                != Some(output.snapshot_input.build_configuration_digest.as_str())
            || !target_digests.contains(capability["targetDigest"].as_str().unwrap())
        {
            bail!("capability tuple is not bound to the exact snapshot configuration");
        }
        if !matches!(
            capability["guaranteedEnumeration"].as_str(),
            Some("COMPLETE_IN_SCOPE" | "PARTIAL" | "UNKNOWN")
        ) || !matches!(
            capability["approximation"].as_str(),
            Some("EXACT" | "SOUND_OVER" | "SOUND_UNDER" | "HEURISTIC" | "NOT_APPLICABLE")
        ) {
            bail!("capability coverage contract is invalid");
        }
        let known_boundaries = capability
            .get("knownBoundaryKinds")
            .and_then(Value::as_array)
            .context("capability knownBoundaryKinds must be an array")?;
        let mut previous = None;
        for kind in known_boundaries {
            let kind = kind
                .as_str()
                .context("known boundary kind must be a string")?;
            if previous.is_some_and(|prior: &str| prior >= kind) {
                bail!("knownBoundaryKinds must be strictly sorted and unique");
            }
            previous = Some(kind);
        }
    }
    Ok(())
}

fn validate_facts_and_boundaries(output: &AdapterOutput) -> Result<()> {
    let capability_grades = output
        .capability_descriptors
        .iter()
        .map(|capability| {
            Ok((
                capability
                    .get("operationUri")
                    .and_then(Value::as_str)
                    .context("capability misses operationUri")?,
                capability
                    .get("grade")
                    .and_then(Value::as_str)
                    .context("capability misses grade")?,
            ))
        })
        .collect::<Result<std::collections::BTreeMap<_, _>>>()?;
    let mut facts = BTreeSet::new();
    for fact in &output.facts {
        let id = fact
            .get("factId")
            .and_then(Value::as_str)
            .context("fact misses factId")?;
        require_digest(id)?;
        if !facts.insert(id.to_owned()) {
            bail!("duplicate factId");
        }
        let relation = fact
            .get("relation")
            .and_then(Value::as_str)
            .context("fact misses relation")?;
        let fact_grade = fact
            .get("grade")
            .and_then(Value::as_str)
            .context("fact misses grade")?;
        if capability_grades.get(relation).copied() != Some(fact_grade) {
            bail!("fact relation/grade is not bound to an exact capability");
        }
        if !matches!(
            fact.get("truth").and_then(Value::as_str),
            Some("TRUE" | "FALSE" | "UNKNOWN")
        ) {
            bail!("fact has invalid truth");
        }
        if fact.get("truth").and_then(Value::as_str) == Some("FALSE")
            && fact.get("enumeration").and_then(Value::as_str) != Some("COMPLETE_IN_SCOPE")
        {
            bail!("negative fact is unjustified without complete enumeration");
        }
    }
    let mut boundaries = BTreeSet::new();
    for boundary in &output.boundaries {
        let id = boundary
            .get("boundaryId")
            .and_then(Value::as_str)
            .context("boundary misses boundaryId")?;
        require_digest(id)?;
        if !boundaries.insert(id.to_owned()) {
            bail!("duplicate boundaryId");
        }
        if !matches!(
            boundary.get("consequence").and_then(Value::as_str),
            Some("LOCAL_ONLY" | "ENUMERATION_INCOMPLETE" | "PROOF_INVALID")
        ) {
            bail!("boundary has an unknown consequence");
        }
    }
    Ok(())
}

fn validate_compiler_receipt(output: &AdapterOutput) -> Result<()> {
    if output
        .compiler_receipt
        .get("status")
        .and_then(Value::as_str)
        != Some("ACCEPTED")
        || output.compiler_receipt.get("grade").and_then(Value::as_str) != Some("COMPILER_CHECKED")
        || output
            .compiler_receipt
            .get("snapshotTreeDigest")
            .and_then(Value::as_str)
            != Some(output.snapshot_input.repository_tree_digest.as_str())
    {
        bail!("compiler receipt is absent or not bound to the snapshot");
    }
    Ok(())
}

fn validate_impact(output: &AdapterOutput) -> Result<()> {
    let status = output
        .impact
        .get("status")
        .and_then(Value::as_str)
        .context("impact result misses status")?;
    if status == "COMPLETE_IN_SCOPE"
        && (output.boundaries.iter().any(|boundary| {
            matches!(
                boundary.get("consequence").and_then(Value::as_str),
                Some("ENUMERATION_INCOMPLETE" | "PROOF_INVALID")
            )
        }) || output.capability_descriptors.iter().any(|capability| {
            capability
                .get("guaranteedEnumeration")
                .and_then(Value::as_str)
                != Some("COMPLETE_IN_SCOPE")
        }))
    {
        bail!("impact falsely claims completeness across a mandatory boundary");
    }
    let boundary_count = output
        .impact
        .get("boundaries")
        .and_then(Value::as_array)
        .map_or(0, Vec::len);
    let obligation_count = output
        .impact
        .get("mandatoryObligations")
        .and_then(Value::as_array)
        .map_or(0, Vec::len);
    if boundary_count > 0 && obligation_count < boundary_count {
        bail!("impact omitted mandatory boundary obligations");
    }
    Ok(())
}

fn semantic_output_digest(output: &AdapterOutput) -> Result<String> {
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

// The projection deliberately binds every measured artifact explicitly; a
// parameter object would merely move this one-shot call's contract elsewhere.
#[allow(clippy::too_many_arguments)]
fn build_projection(
    output: &AdapterOutput,
    adapter_path: &Path,
    runtime: &RuntimeIdentity,
    args: &RunArgs,
    cold_wall_micros: u64,
    raw_validation_micros: u64,
    adapter_object: &StoredObject,
    store_write_micros: u64,
    store_read_micros: u64,
    warm_wall_micros: Option<u64>,
    warm_adapter_cost: Option<&evidence_adapters::CostRecord>,
    core_binding: &CoreBindingSummary,
) -> Result<Value> {
    let affected = output
        .impact
        .get("affected")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let paths = output
        .impact
        .get("paths")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let obligations = output
        .impact
        .get("mandatoryObligations")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let affected_ids = affected
        .iter()
        .filter_map(|entry| entry.get("entityId").and_then(Value::as_str))
        .collect::<BTreeSet<_>>();
    let selected_entities = output
        .entities
        .iter()
        .filter(|entity| {
            affected_ids.is_empty()
                || entity
                    .get("opaqueId")
                    .and_then(Value::as_str)
                    .is_some_and(|id| affected_ids.contains(id))
        })
        .take(args.max_entities)
        .map(|entity| {
            json!({
                "entityId":entity.get("opaqueId"),
                "resolution":entity.get("resolution"),
                "coarseKind":entity.get("coarseKind"),
                "displayName":entity.get("displayName"),
                "primaryDefinition":entity.get("primaryDefinition"),
            })
        })
        .collect::<Vec<_>>();
    let path_fact_ids = paths
        .iter()
        .filter_map(|path| path.get("factId").and_then(Value::as_str))
        .collect::<BTreeSet<_>>();
    let relevant_relations = output
        .facts
        .iter()
        .filter(|fact| {
            path_fact_ids.is_empty()
                || fact
                    .get("factId")
                    .and_then(Value::as_str)
                    .is_some_and(|id| path_fact_ids.contains(id))
        })
        .take(args.max_entities)
        .map(|fact| {
            json!({
                "factId":fact.get("factId"),
                "relation":fact.get("relation"),
                "owner":fact.get("owner"),
                "target":fact.get("target"),
                "truth":fact.get("truth"),
                "grade":fact.get("grade"),
                "enumeration":fact.get("enumeration"),
                "range":fact.get("range"),
            })
        })
        .collect::<Vec<_>>();
    let boundary_summaries = output
        .boundaries
        .iter()
        .map(|boundary| {
            json!({
                "boundaryId":boundary.get("boundaryId"),
                "kindUri":boundary.get("kindUri"),
                "consequence":boundary.get("consequence"),
                "origin":boundary.get("origin"),
            })
        })
        .collect::<Vec<_>>();
    let capabilities = output
        .capability_descriptors
        .iter()
        .map(|capability| {
            json!({
                "operationUri":capability.get("operationUri"),
                "grade":capability.get("grade"),
                "support":capability.get("support"),
                "guaranteedEnumeration":capability.get("guaranteedEnumeration"),
            })
        })
        .collect::<Vec<_>>();
    Ok(json!({
        "schema":PROJECTION_SCHEMA,
        "query":{
            "seedEntity":args.seed_entity,
            "maxDepth":args.max_depth,
            "maxEntities":args.max_entities,
            "view":"RELATION_IMPACT",
            "sourcePolicy":"ANCHORS_ONLY",
            "adapterArgumentsDigest":canonical_hash(&args.adapter_arg)?,
        },
        "snapshot":{
            "repositoryTreeDigest":output.snapshot_input.repository_tree_digest,
            "vcsRevision":output.snapshot_input.vcs_revision,
            "dirty":output.snapshot_input.dirty,
            "buildModelDigest":output.snapshot_input.build_model_digest,
            "buildConfigurationDigest":output.snapshot_input.build_configuration_digest,
            "dependencyGraphDigest":output.snapshot_input.dependency_graph_digest,
            "generatedSourcesManifestDigest":output.snapshot_input.generated_sources_manifest_digest,
        },
        "adapter":output.adapter,
        "capabilities":capabilities,
        "status":output.impact.get("status"),
        "selectedEntities":selected_entities,
        "relevantRelations":relevant_relations,
        "affected":affected,
        "paths":paths,
        "mandatoryObligations":obligations,
        "boundaries":boundary_summaries,
        "compilerReceipt":output.compiler_receipt,
        "completeness":{
            "status":output.impact.get("status"),
            "factCount":output.facts.len(),
            "boundaryCount":output.boundaries.len(),
            "unknownMandatoryObligationCount":obligations.iter().filter(|obligation| obligation.get("status").and_then(Value::as_str) == Some("UNKNOWN")).count()
        },
        "provenance":{
            "runtime":runtime,
            "adapterRealPath":adapter_path,
            "adapterBinaryDigest":output.adapter.binary_digest,
            "adapterOutputDigest":output.output_digest,
            "adapterEnvelopeFileDigest":adapter_object.digest,
            "adapterOutputObject":adapter_object,
            "semanticOutputDigest":semantic_output_digest(output)?,
            "evidenceCore":core_binding,
        },
        "cost":{
            "coldAdapterInvocationWallMicros":cold_wall_micros,
            "warmAdapterInvocationWallMicros":warm_wall_micros,
            "rawEnvelopeValidationMicros":raw_validation_micros,
            "evidenceStoreWriteMicros":store_write_micros,
            "evidenceStoreReadMicros":store_read_micros,
            "coldAdapter":output.cost,
            "warmAdapter":warm_adapter_cost,
            "projectionBytes":0,
            "modelVisibleSourceBytes":0,
            "repetitions":args.repetitions
        },
        "projectionDigest":""
    }))
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct RuntimeIdentity {
    real_path: PathBuf,
    binary_digest: String,
}

fn runtime_identity() -> Result<RuntimeIdentity> {
    let lexical = std::env::current_exe()?;
    let lexical_metadata = std::fs::symlink_metadata(&lexical)?;
    if !lexical_metadata.is_file() || lexical_metadata.file_type().is_symlink() {
        bail!("evidence runtime must be a regular non-symlink executable");
    }
    let real_path = lexical.canonicalize()?;
    let binary_digest = hash_bytes(&std::fs::read(&real_path)?);
    Ok(RuntimeIdentity {
        real_path,
        binary_digest,
    })
}

#[derive(Debug, serde::Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct StoredObject {
    digest: String,
    relative_path: String,
    size_bytes: u64,
}

struct EvidenceStore {
    root: PathBuf,
}

impl EvidenceStore {
    fn open(path: &Path) -> Result<Self> {
        if !path.is_absolute() {
            bail!("evidence store path must be absolute");
        }
        let lexical = std::fs::symlink_metadata(path)?;
        if !lexical.is_dir() || lexical.file_type().is_symlink() {
            bail!("evidence store must be an existing regular non-symlink directory");
        }
        let root = path.canonicalize()?;
        let objects = root.join("objects").join("sha256");
        std::fs::create_dir_all(&objects)?;
        let objects_metadata = std::fs::symlink_metadata(&objects)?;
        if !objects_metadata.is_dir() || objects_metadata.file_type().is_symlink() {
            bail!("evidence store object directory is unsafe");
        }
        Ok(Self { root })
    }

    fn object_path(&self, digest: &str) -> Result<(String, PathBuf)> {
        let normalized = normalize_digest(digest)?;
        let hex = normalized.strip_prefix("sha256:").unwrap();
        let relative = format!("objects/sha256/{hex}.json");
        Ok((relative.clone(), self.root.join(relative)))
    }

    fn publish(&self, bytes: &[u8], expected_digest: &str) -> Result<StoredObject> {
        if hash_bytes(bytes) != normalize_digest(expected_digest)? {
            bail!("evidence object bytes do not match declared digest");
        }
        let (relative_path, path) = self.object_path(expected_digest)?;
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(mut file) => {
                file.write_all(bytes)?;
                file.sync_all()?;
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error.into()),
        }
        let read = self.read_verified(expected_digest)?;
        if read != bytes {
            bail!("existing content-addressed object differs from requested bytes");
        }
        Ok(StoredObject {
            digest: normalize_digest(expected_digest)?,
            relative_path,
            size_bytes: bytes.len() as u64,
        })
    }

    fn read_verified(&self, digest: &str) -> Result<Vec<u8>> {
        let (_, path) = self.object_path(digest)?;
        let metadata = std::fs::symlink_metadata(&path)?;
        if !metadata.is_file() || metadata.file_type().is_symlink() {
            bail!("evidence object must be a regular non-symlink file");
        }
        let bytes = std::fs::read(&path)?;
        if hash_bytes(&bytes) != normalize_digest(digest)? {
            bail!("content-addressed evidence object digest mismatch");
        }
        Ok(bytes)
    }
}

fn enforce_budget(projection: &mut Value, max_bytes: usize) -> Result<()> {
    if max_bytes < 1024 {
        bail!("maxProjectionBytes must be at least 1024");
    }
    let target_bytes = max_bytes.saturating_sub(256);
    let truncated = canonical_bytes(projection)?.len() > target_bytes;
    if truncated {
        projection["status"] = Value::String("PARTIAL_BUDGET".to_owned());
        projection["completeness"]["status"] = Value::String("PARTIAL_BUDGET".to_owned());
        let budget_boundary = json!({
            "boundaryId":canonical_hash(&json!({"kind":"projection-budget","maxBytes":max_bytes}))?,
            "kindUri":"codeclew.boundary/projection-budget/1",
            "consequence":"ENUMERATION_INCOMPLETE",
            "origin":Value::Null,
        });
        if let Some(boundaries) = projection
            .get_mut("boundaries")
            .and_then(Value::as_array_mut)
        {
            boundaries.insert(0, budget_boundary);
        }
    }
    for field in [
        "mandatoryObligations",
        "boundaries",
        "capabilities",
        "relevantRelations",
        "selectedEntities",
        "paths",
        "affected",
    ] {
        while canonical_bytes(projection)?.len() > target_bytes {
            let Some(array) = projection.get_mut(field).and_then(Value::as_array_mut) else {
                break;
            };
            let minimum = usize::from(field == "boundaries" && truncated);
            if array.len() <= minimum {
                break;
            }
            array.pop();
        }
    }
    projection["cost"]["projectionBytes"] = Value::from(canonical_bytes(projection)?.len() as u64);
    if canonical_bytes(projection)?.len() > max_bytes {
        bail!("projection metadata alone exceeds maxProjectionBytes");
    }
    Ok(())
}

fn canonical_directory(path: &Path) -> Result<PathBuf> {
    if !path.is_absolute() {
        bail!("repository path must be absolute");
    }
    let canonical = path.canonicalize()?;
    let metadata = std::fs::symlink_metadata(&canonical)?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        bail!("repository must be a regular non-symlink directory");
    }
    Ok(canonical)
}

fn pinned_executable(path: &Path, expected: &str) -> Result<PathBuf> {
    if !path.is_absolute() {
        bail!("adapter path must be absolute");
    }
    let lexical = std::fs::symlink_metadata(path)?;
    if !lexical.is_file() || lexical.file_type().is_symlink() {
        bail!("adapter must be a regular non-symlink file");
    }
    let canonical = path.canonicalize()?;
    let bytes = std::fs::read(&canonical)?;
    if hash_bytes(&bytes) != normalize_digest(expected)? {
        bail!("adapter binary digest differs from --adapter-sha256");
    }
    Ok(canonical)
}

fn normalize_digest(value: &str) -> Result<String> {
    let normalized = value.strip_prefix("sha256:").unwrap_or(value);
    if normalized.len() != 64
        || !normalized
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        bail!("digest must be 64 lowercase hexadecimal SHA-256 bytes");
    }
    Ok(format!("sha256:{normalized}"))
}

fn require_digest(value: &str) -> Result<()> {
    normalize_digest(value).map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;
    use evidence_adapters::{AdapterIdentity, CostRecord, SnapshotInput};

    fn output() -> AdapterOutput {
        let tree = hash_bytes(b"tree");
        let config = hash_bytes(b"config");
        let toolchain = hash_bytes(b"toolchain");
        let target = hash_bytes(b"target");
        let boundary_id = hash_bytes(b"boundary");
        let mut output = AdapterOutput {
            schema: ADAPTER_OUTPUT_SCHEMA.to_owned(),
            adapter: AdapterIdentity {
                adapter_id: "test.adapter".to_owned(),
                version: "1".to_owned(),
                binary_digest: hash_bytes(b"adapter"),
                language_id: "opaque-language".to_owned(),
            },
            snapshot_input: SnapshotInput {
                repository_tree_digest: tree.clone(),
                vcs_revision: None,
                dirty: false,
                sources: vec![evidence_adapters::SourceInput {
                    artifact_id: "source:src/main.opaque".to_owned(),
                    normalized_path: "src/main.opaque".to_owned(),
                    content_digest: hash_bytes(b"source"),
                    size_bytes: 6,
                    origin: "USER".to_owned(),
                }],
                build_system_uri: "build:test".to_owned(),
                build_model_digest: hash_bytes(b"model"),
                build_configuration_digest: config.clone(),
                dependency_graph_digest: hash_bytes(b"deps"),
                toolchain: json!({
                    "toolUri":"tool:test/1",
                    "version":"1",
                    "distributionDigest":toolchain,
                }),
                targets: vec![json!({
                    "targetId":"target:test",
                    "configurationDigest":target,
                    "enabledFeatures":[],
                    "platform":"test-platform",
                    "compilerFlags":[],
                })],
                relevant_environment: vec![],
                generated_sources_manifest_digest: hash_bytes(b"generated"),
            },
            capability_descriptors: vec![json!({
                "operationUri":"relation:test/1",
                "operationVersion":"1",
                "operationSpecificationDigest":hash_bytes(b"relation:test/1"),
                "languageId":"opaque-language",
                "adapterId":"test.adapter",
                "adapterVersion":"1",
                "toolchainDigest":toolchain,
                "buildConfigurationDigest":config,
                "targetDigest":target,
                "grade":"NAVIGATION",
                "support":"SUPPORTED",
                "guaranteedEnumeration":"PARTIAL",
                "approximation":"SOUND_UNDER",
                "knownBoundaryKinds":["boundary:test/1"],
                "costClass":"codeclew.cost/test/1",
            })],
            entities: vec![],
            occurrences: vec![],
            facts: vec![],
            boundaries: vec![json!({
                "boundaryId":boundary_id,
                "kindUri":"boundary:test/1",
                "consequence":"ENUMERATION_INCOMPLETE",
                "origin":Value::Null,
            })],
            compiler_receipt: json!({
                "schema":"codeclew.compiler-receipt/0.1",
                "method":"test-compiler",
                "status":"ACCEPTED",
                "grade":"COMPILER_CHECKED",
                "snapshotTreeDigest":tree,
                "claim":"test sources accepted",
            }),
            impact: json!({
                "schema":"codeclew.impact-result/0.1",
                "status":"PARTIAL_BOUNDARY",
                "reason":"TEST_BOUNDARY",
                "closureSpecification":evidence_adapters::IMPACT_CLOSURE_SPEC,
                "affected":[],
                "paths":[],
                "boundaries":[{}],
                "mandatoryObligations":[{
                    "id":"validate-test-boundary",
                    "kind":"codeclew.obligation/validate-boundary/1",
                    "mandatory":true,
                    "status":"UNKNOWN",
                    "reason":"TEST_BOUNDARY",
                }]
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

    #[test]
    fn incomplete_boundary_cannot_be_called_complete() {
        let mut output = output();
        output.impact["status"] = Value::String("COMPLETE_IN_SCOPE".to_owned());
        output.seal().unwrap();
        assert!(validate_output(&output, &output.adapter.binary_digest).is_err());
    }

    #[test]
    fn language_is_not_a_decision_branch() {
        let output = output();
        validate_output(&output, &output.adapter.binary_digest).unwrap();
        let binding = validate_core_binding(&output).unwrap();
        assert_eq!(binding.capability_count, 2);
        assert_eq!(binding.fact_count, 0);
        assert_eq!(binding.obligation_graph_count, 1);
    }

    #[test]
    fn capability_binding_mutation_is_rejected_by_both_layers() {
        let mut output = output();
        output.capability_descriptors[0]["toolchainDigest"] =
            Value::String(hash_bytes(b"other-toolchain"));
        output.seal().unwrap();
        assert!(validate_output(&output, &output.adapter.binary_digest).is_err());
        assert!(validate_core_binding(&output).is_err());
    }

    #[test]
    fn opaque_adapter_arguments_accept_tool_flags_without_language_dispatch() {
        let cli = Cli::try_parse_from([
            "codeclew-evidence",
            "run",
            "--repo=/tmp/repo",
            "--adapter=/tmp/adapter",
            "--adapter-sha256=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "--store=/tmp/store",
            "--adapter-arg=--provider-tool=/tmp/tool",
            "--adapter-arg=--allow-trusted-workspace-analysis",
        ])
        .unwrap();
        let EvidenceCommand::Run(args) = cli.command;
        assert_eq!(
            args.adapter_arg,
            [
                "--provider-tool=/tmp/tool",
                "--allow-trusted-workspace-analysis"
            ]
        );
    }

    #[test]
    fn content_addressed_store_is_idempotent_and_rejects_mutation() {
        let directory = tempfile::tempdir().unwrap();
        let store = EvidenceStore::open(directory.path()).unwrap();
        let bytes = br#"{"schema":"test"}"#;
        let digest = hash_bytes(bytes);
        let first = store.publish(bytes, &digest).unwrap();
        let second = store.publish(bytes, &digest).unwrap();
        assert_eq!(first.digest, second.digest);
        assert_eq!(store.read_verified(&digest).unwrap(), bytes);

        let path = directory.path().join(&first.relative_path);
        std::fs::write(&path, b"mutated").unwrap();
        assert!(store.read_verified(&digest).is_err());
        assert!(store.publish(bytes, &digest).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn content_addressed_store_rejects_symlink_object() {
        use std::os::unix::fs::symlink;

        let directory = tempfile::tempdir().unwrap();
        let store = EvidenceStore::open(directory.path()).unwrap();
        let bytes = b"object";
        let digest = hash_bytes(bytes);
        let (_, path) = store.object_path(&digest).unwrap();
        let target = directory.path().join("target.json");
        std::fs::write(&target, bytes).unwrap();
        symlink(&target, &path).unwrap();
        assert!(store.read_verified(&digest).is_err());
        assert!(store.publish(bytes, &digest).is_err());
    }

    #[test]
    fn weak_negative_fact_is_rejected() {
        let mut output = output();
        output.facts.push(json!({
            "factId":hash_bytes(b"fact"),
            "truth":"FALSE",
            "enumeration":"PARTIAL"
        }));
        output.seal().unwrap();
        assert!(validate_output(&output, &output.adapter.binary_digest).is_err());
    }
}
