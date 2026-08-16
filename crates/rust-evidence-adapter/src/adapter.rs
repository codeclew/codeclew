use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::env;
use std::ffi::OsStr;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Output};
use std::time::Instant;

use anyhow::{Context, Result, anyhow, bail, ensure};
use protobuf::Message;
use scip::types::{self as scip_types, Index as ScipIndex, occurrence};
use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use tempfile::TempDir;
use walkdir::{DirEntry, WalkDir};

use crate::protocol::{
    ADAPTER_SCHEMA, AdapterIdentity, AdapterOutput, Boundary, BoundaryDetails,
    CapabilityDescriptor, CargoTarget, CompilerReceipt, CompilerReceiptProviderPayload, CostRecord,
    Entity, EnvironmentInput, EvidenceRange, Fact, FactProviderPayload, ImpactAffected,
    ImpactOutput, ImpactPath, InvocationReceipt, Location, MandatoryObligation, Occurrence,
    SnapshotInput, SourceArtifact, SourceRange, StableInvocationReceipt, TargetDescriptor,
    TargetProviderPayload, ToolIdentity, ToolchainInput, ToolchainProviderPayload,
    canonical_digest, canonical_json, sha256_bytes,
};

const ADAPTER_ID: &str = "codeclew.rust-evidence-adapter";
const ADAPTER_VERSION: &str = env!("CARGO_PKG_VERSION");
const NAVIGATION_OPERATION: &str = "urn:codeclew:operation:resolved-navigation";
const CHECK_OPERATION: &str = "urn:codeclew:operation:compiler-check";
const IMPACT_OPERATION: &str = "urn:codeclew:operation:bounded-may-impact";
const RELATIONSHIP_OPERATION: &str = "urn:codeclew:relation:scip-symbol-relationship";
const DOCUMENT_OCCURRENCE_OPERATION: &str =
    "urn:codeclew:relation:document-has-resolved-occurrence";
const OPERATION_VERSION: &str = "0.1";
const HASH_ENVIRONMENT_NAMES: &[&str] = &[
    "CARGO_BUILD_TARGET",
    "CARGO_ENCODED_RUSTFLAGS",
    "CARGO_HOME",
    "CARGO_NET_OFFLINE",
    "CARGO_TARGET_DIR",
    "HOST",
    "RUSTC_BOOTSTRAP",
    "RUSTFLAGS",
    "RUSTUP_TOOLCHAIN",
    "TARGET",
];

#[derive(Debug, Clone)]
pub struct CollectConfig {
    pub repo: PathBuf,
    pub rust_analyzer: PinnedExecutable,
    pub cargo: PinnedExecutable,
    pub rustc: PinnedExecutable,
    pub git: PinnedExecutable,
    pub seed_entity: Option<String>,
    pub max_depth: u32,
    pub max_entities: u32,
    pub allow_trusted_workspace_code_execution: bool,
}

#[derive(Debug, Clone)]
pub struct PinnedExecutable {
    pub path: PathBuf,
    pub expected_sha256: String,
}

#[derive(Debug, Clone)]
struct VerifiedTool {
    tool_id: String,
    requested_path: String,
    canonical_path: PathBuf,
    expected_digest: String,
    observed_digest: String,
    binary_bytes: u64,
}

#[derive(Debug)]
struct RepositoryManifest {
    tree_digest: String,
    repository_bytes: u64,
    source_bytes: u64,
    sources: Vec<SourceArtifact>,
    config_artifacts: Vec<SourceArtifact>,
}

#[derive(Debug)]
struct CommandResult {
    output: Output,
    receipt: InvocationReceipt,
}

#[derive(Debug, Clone)]
struct ParsedOccurrence {
    output: Occurrence,
}

#[derive(Debug, Clone)]
struct EntityMetadata {
    display_name: Option<String>,
    kind_hint: String,
    language_payload: BTreeMap<String, String>,
}

#[derive(Debug)]
struct SourceEvidence {
    artifact_id: String,
    content_digest: String,
    bytes: Vec<u8>,
    line_starts: Vec<usize>,
}

#[derive(Debug)]
struct ParsedScip {
    entities: Vec<Entity>,
    occurrences: Vec<Occurrence>,
    facts: Vec<Fact>,
}

#[derive(Debug, Clone, Copy)]
struct ImpactQuery<'a> {
    requested_seed: Option<&'a str>,
    max_depth: u32,
    max_entities: u32,
}

pub fn collect(config: &CollectConfig) -> Result<AdapterOutput> {
    ensure!(
        config.allow_trusted_workspace_code_execution,
        "collection refused: cargo and rust-analyzer can execute build scripts and procedural macros; pass the explicit trusted-workspace acknowledgement"
    );
    ensure!(
        config.max_entities > 0,
        "max-entities must be greater than zero"
    );

    let total_started = Instant::now();
    let repo = config
        .repo
        .canonicalize()
        .with_context(|| format!("canonicalize repository {}", config.repo.display()))?;
    ensure!(
        repo.join("Cargo.toml").is_file(),
        "repository has no Cargo.toml"
    );

    let mut phases = BTreeMap::new();
    let mut invocations = Vec::new();

    let phase = Instant::now();
    let rust_analyzer = verify_tool("rust-analyzer", &config.rust_analyzer)?;
    let cargo = verify_tool("cargo", &config.cargo)?;
    let rustc = verify_tool("rustc", &config.rustc)?;
    let git = verify_tool("git", &config.git)?;
    phases.insert("verifyPinnedExecutables".to_owned(), elapsed_micros(phase));

    let execution_environment = execution_environment(&cargo, &rustc);
    let phase = Instant::now();
    let (rust_analyzer_identity, rust_analyzer_version) = tool_identity(
        &rust_analyzer,
        &["--version"],
        &repo,
        &execution_environment,
        None,
        &mut invocations,
    )?;
    let (cargo_identity, cargo_version) = tool_identity(
        &cargo,
        &["--version", "--verbose"],
        &repo,
        &execution_environment,
        None,
        &mut invocations,
    )?;
    let (mut rustc_identity, rustc_version) = tool_identity(
        &rustc,
        &["--version", "--verbose"],
        &repo,
        &execution_environment,
        None,
        &mut invocations,
    )?;
    let (git_identity, _) = tool_identity(
        &git,
        &["--version"],
        &repo,
        &execution_environment,
        None,
        &mut invocations,
    )?;

    let sysroot_result = run_command(
        &rustc,
        &["--print", "sysroot"],
        &repo,
        &execution_environment,
    )?;
    ensure_command_success("rustc --print sysroot", &sysroot_result)?;
    let sysroot_text = String::from_utf8(sysroot_result.output.stdout.clone())?;
    let sysroot = PathBuf::from(sysroot_text.trim())
        .canonicalize()
        .context("canonicalize rustc sysroot")?;
    invocations.push(sysroot_result.receipt);
    let (sysroot_digest, sysroot_bytes) = hash_directory(&sysroot)?;
    rustc_identity.distribution_digest = Some(sysroot_digest.clone());
    phases.insert("captureToolchain".to_owned(), elapsed_micros(phase));

    let phase = Instant::now();
    let before = repository_manifest(&repo)?;
    phases.insert("snapshotBefore".to_owned(), elapsed_micros(phase));

    let phase = Instant::now();
    let vcs_revision = git_optional(
        &git,
        &["rev-parse", "HEAD"],
        &repo,
        &execution_environment,
        &mut invocations,
    )?;
    let git_status = run_command(
        &git,
        &[
            "status",
            "--porcelain=v1",
            "--untracked-files=all",
            "--",
            ".",
        ],
        &repo,
        &execution_environment,
    )?;
    let dirty = !git_status.output.status.success() || !git_status.output.stdout.is_empty();
    invocations.push(git_status.receipt);
    phases.insert("captureVcs".to_owned(), elapsed_micros(phase));

    let phase = Instant::now();
    let metadata_result = run_command(
        &cargo,
        &["metadata", "--format-version", "1", "--locked"],
        &repo,
        &execution_environment,
    )?;
    ensure_command_success("cargo metadata", &metadata_result)?;
    let metadata: Value = serde_json::from_slice(&metadata_result.output.stdout)
        .context("parse cargo metadata JSON")?;
    let metadata_receipt = metadata_result.receipt.clone();
    invocations.push(metadata_result.receipt);
    let build_model_digest = canonical_digest(&metadata)?;
    let dependency_graph_digest = dependency_graph_digest(&metadata)?;
    let cargo_targets = metadata_targets(&metadata, &repo)?;
    let relevant_environment = relevant_environment();
    let build_configuration_digest = build_configuration_digest(
        &before.config_artifacts,
        &relevant_environment,
        &metadata_receipt.environment_digest,
    )?;
    phases.insert("cargoMetadata".to_owned(), elapsed_micros(phase));

    let phase = Instant::now();
    let check_result = run_command(
        &cargo,
        &[
            "check",
            "--workspace",
            "--all-targets",
            "--locked",
            "--message-format=json",
        ],
        &repo,
        &execution_environment,
    )?;
    let check_receipt = check_result.receipt.clone();
    let (diagnostics_digest, diagnostic_count) = cargo_diagnostics(&check_result.output.stdout)?;
    ensure_command_success("cargo check", &check_result)?;
    invocations.push(check_result.receipt);
    phases.insert("cargoCheck".to_owned(), elapsed_micros(phase));

    let phase = Instant::now();
    let temp = TempDir::new().context("create SCIP output directory")?;
    let scip_path = temp.path().join("index.scip");
    let scip_path_arg = scip_path
        .to_str()
        .ok_or_else(|| anyhow!("SCIP output path is not UTF-8"))?;
    let repo_arg = repo
        .to_str()
        .ok_or_else(|| anyhow!("repository path is not UTF-8"))?;
    let scip_result = run_command(
        &rust_analyzer,
        &["scip", repo_arg, "--output", scip_path_arg],
        &repo,
        &execution_environment,
    )?;
    ensure_command_success("rust-analyzer scip", &scip_result)?;
    invocations.push(scip_result.receipt);
    let scip_bytes = fs::read(&scip_path).context("read rust-analyzer SCIP output")?;
    let scip_artifact_bytes = scip_bytes.len() as u64;
    let scip_artifact_digest = sha256_bytes(&scip_bytes);
    let scip_index = ScipIndex::parse_from_bytes(&scip_bytes).context("parse SCIP protobuf")?;
    phases.insert("rustAnalyzerScip".to_owned(), elapsed_micros(phase));

    let phase = Instant::now();
    let scip_evidence_id = canonical_digest(&(
        &scip_artifact_digest,
        &rust_analyzer.observed_digest,
        &build_configuration_digest,
        &before.tree_digest,
    ))?;
    let parsed = parse_scip(&scip_index, &repo, &before.sources, &scip_evidence_id)?;
    phases.insert("parseScip".to_owned(), elapsed_micros(phase));

    let phase = Instant::now();
    let after = repository_manifest(&repo)?;
    ensure!(
        before.tree_digest == after.tree_digest,
        "repository snapshot changed during collection (before {}, after {}); evidence discarded",
        before.tree_digest,
        after.tree_digest
    );
    phases.insert("snapshotAfter".to_owned(), elapsed_micros(phase));

    let boundaries = rust_boundaries(vcs_revision.is_none());
    let source_manifest_digest = canonical_digest(&before.sources)?;
    let generated_sources_manifest_digest = canonical_digest(&Vec::<SourceArtifact>::new())?;
    let enabled_features = enabled_features(&metadata)?;
    let platform = configured_platform(&rustc_version);
    let compiler_flags = vec![
        "--workspace".to_owned(),
        "--all-targets".to_owned(),
        "--locked".to_owned(),
    ];
    let target_configuration_digest = canonical_digest(&serde_json::json!({
        "buildConfigurationDigest":build_configuration_digest,
        "cargoTargets":cargo_targets,
        "enabledFeatures":enabled_features,
        "platform":platform,
        "compilerFlags":compiler_flags,
    }))?;
    let targets = vec![TargetDescriptor {
        target_id: "cargo-workspace-all-targets".to_owned(),
        configuration_digest: target_configuration_digest.clone(),
        enabled_features,
        platform,
        compiler_flags,
        provider_payload: TargetProviderPayload {
            cargo_targets,
            scope: "cargo check --workspace --all-targets --locked".to_owned(),
        },
    }];
    let tools = vec![
        rust_analyzer_identity,
        cargo_identity,
        rustc_identity,
        git_identity,
    ];
    let toolchain_distribution_digest = canonical_digest(&serde_json::json!({
        "tools":tools,
        "rustcSysrootDigest":sysroot_digest,
    }))?;
    let snapshot = SnapshotInput {
        repository_tree_digest: before.tree_digest.clone(),
        vcs_revision,
        dirty,
        sources: before.sources,
        build_system_uri: "https://doc.rust-lang.org/cargo/".to_owned(),
        build_model_digest,
        build_configuration_digest: build_configuration_digest.clone(),
        dependency_graph_digest,
        toolchain: ToolchainInput {
            tool_uri: "urn:codeclew:toolchain:rust:cargo-rustc-rust-analyzer".to_owned(),
            version: format!(
                "rustc={}; cargo={}; rust-analyzer={}",
                first_line(&rustc_version),
                first_line(&cargo_version),
                first_line(&rust_analyzer_version)
            ),
            distribution_digest: toolchain_distribution_digest.clone(),
            provider_payload: ToolchainProviderPayload {
                language: "rust".to_owned(),
                tools,
                rustc_sysroot_bytes_hashed: sysroot_bytes,
                generated_sources_completeness: "UNKNOWN".to_owned(),
                repository_tree_exclusions: vec![
                    ".git/**".to_owned(),
                    "target/** and nested target/** directories".to_owned(),
                ],
                scip_artifact_digest,
                inherited_environment_digest: metadata_receipt.environment_digest.clone(),
                telemetry_limitations: vec![
                    "peak RSS is not observed".to_owned(),
                    "filesystem and Cargo cache warmth are not controlled".to_owned(),
                    "prior cache construction cost is not attributed".to_owned(),
                ],
                provider_invocations: Vec::new(),
            },
        },
        targets,
        relevant_environment,
        generated_sources_manifest_digest,
    };
    let compiler_receipt = CompilerReceipt {
        schema: "codeclew.compiler-receipt/0.1".to_owned(),
        method: "cargo check --workspace --all-targets --locked".to_owned(),
        status: if check_receipt.success { "ACCEPTED" } else { "REJECTED" }.to_owned(),
        grade: "COMPILER_CHECKED".to_owned(),
        snapshot_tree_digest: before.tree_digest.clone(),
        claim: "configured Rust workspace is accepted by the pinned cargo/rustc invocation; this is not a behavioural proof".to_owned(),
        provider_payload: CompilerReceiptProviderPayload {
            configured_scope: vec![
                "cargo check --workspace --all-targets --locked".to_owned(),
                "current target/toolchain/environment only".to_owned(),
            ],
            invocation: StableInvocationReceipt::from_invocation(
                &check_receipt,
                diagnostics_digest,
                diagnostic_count,
            ),
            source_tree_digest_before: before.tree_digest.clone(),
            source_tree_digest_after: after.tree_digest,
        },
    };
    let mut known_boundary_kinds = boundaries
        .iter()
        .map(|boundary| boundary.kind_uri.clone())
        .collect::<Vec<_>>();
    known_boundary_kinds.sort();
    known_boundary_kinds.dedup();
    let capability_descriptors = capabilities(
        &build_configuration_digest,
        &toolchain_distribution_digest,
        &target_configuration_digest,
        &known_boundary_kinds,
    );
    let impact = may_impact(
        &parsed.entities,
        &parsed.occurrences,
        &parsed.facts,
        &boundaries,
        ImpactQuery {
            requested_seed: config.seed_entity.as_deref(),
            max_depth: config.max_depth,
            max_entities: config.max_entities,
        },
    )?;

    let source_bytes_hashed = before.source_bytes + after.source_bytes;
    let _repository_bytes_hashed = before.repository_bytes + after.repository_bytes;
    let _toolchain_bytes_hashed = sysroot_bytes
        + rust_analyzer.binary_bytes
        + cargo.binary_bytes
        + rustc.binary_bytes
        + git.binary_bytes;
    let _subprocess_stdout_bytes: u64 = invocations.iter().map(|item| item.stdout_bytes).sum();
    let _subprocess_stderr_bytes: u64 = invocations.iter().map(|item| item.stderr_bytes).sum();
    let payload_measure = canonical_json(&(
        &parsed.entities,
        &parsed.occurrences,
        &parsed.facts,
        &boundaries,
        &compiler_receipt,
        &impact,
    ))?
    .len() as u64;
    phases.insert("assembleEnvelope".to_owned(), 0);
    let cost = CostRecord {
        total_wall_micros: elapsed_micros(total_started),
        repository_snapshot_micros: phases.get("snapshotBefore").copied().unwrap_or(0)
            + phases.get("snapshotAfter").copied().unwrap_or(0),
        build_discovery_micros: phases.get("cargoMetadata").copied().unwrap_or(0),
        cold_index_micros: phases.get("rustAnalyzerScip").copied().unwrap_or(0),
        warm_index_micros: 0,
        adapter_micros: phases.get("parseScip").copied().unwrap_or(0),
        query_micros: impact.query_micros,
        source_bytes_read: source_bytes_hashed,
        emitted_bytes: 0,
        stored_fact_bytes: payload_measure + scip_artifact_bytes,
        model_visible_source_bytes: 0,
        cache_requests: 0,
        cache_hits: 0,
        provider_processing_micros: phases.values().sum(),
    };

    let current_exe = env::current_exe().context("resolve adapter executable")?;
    let adapter = AdapterIdentity {
        adapter_id: ADAPTER_ID.to_owned(),
        version: ADAPTER_VERSION.to_owned(),
        binary_digest: hash_file(&current_exe)?.0,
        language_id: "rust".to_owned(),
    };
    let mut output = AdapterOutput {
        schema: ADAPTER_SCHEMA.to_owned(),
        adapter,
        snapshot_input: snapshot,
        capability_descriptors,
        entities: parsed.entities,
        occurrences: parsed.occurrences,
        facts: parsed.facts,
        boundaries,
        compiler_receipt,
        impact,
        cost,
        output_digest: String::new(),
    };
    seal_output(&mut output)?;
    validate_output(&output, &source_manifest_digest, &rust_analyzer_version)?;
    Ok(output)
}

pub fn impact_from_output(
    output: &AdapterOutput,
    seed_entity: Option<&str>,
    max_depth: u32,
    max_entities: u32,
) -> Result<ImpactOutput> {
    ensure!(
        output.schema == ADAPTER_SCHEMA,
        "unsupported adapter output schema"
    );
    ensure!(max_entities > 0, "max-entities must be greater than zero");
    verify_output_digest(output)?;
    may_impact(
        &output.entities,
        &output.occurrences,
        &output.facts,
        &output.boundaries,
        ImpactQuery {
            requested_seed: seed_entity,
            max_depth,
            max_entities,
        },
    )
}

pub fn verify_output_digest(output: &AdapterOutput) -> Result<()> {
    let mut unsigned = output.clone();
    let expected = unsigned.output_digest.clone();
    unsigned.output_digest.clear();
    let observed = canonical_digest(&unsigned)?;
    ensure!(expected == observed, "adapter output digest mismatch");
    Ok(())
}

fn verify_tool(tool_id: &str, pin: &PinnedExecutable) -> Result<VerifiedTool> {
    let expected = normalize_expected_digest(&pin.expected_sha256)
        .with_context(|| format!("MISSING_PROVIDER:{tool_id}:invalid expected digest"))?;
    ensure!(
        pin.path.is_absolute(),
        "MISSING_PROVIDER:{tool_id}:executable path must be absolute"
    );
    let canonical_path = pin
        .path
        .canonicalize()
        .with_context(|| format!("MISSING_PROVIDER:{tool_id}:{}", pin.path.display()))?;
    ensure!(
        canonical_path.is_file(),
        "MISSING_PROVIDER:{tool_id}:path is not a regular file"
    );
    let (observed_digest, binary_bytes) = hash_file(&canonical_path)?;
    ensure!(
        observed_digest == expected,
        "MISSING_PROVIDER:{tool_id}:digest mismatch: expected {}, observed {}",
        expected,
        observed_digest
    );
    Ok(VerifiedTool {
        tool_id: tool_id.to_owned(),
        requested_path: pin.path.display().to_string(),
        canonical_path,
        expected_digest: expected,
        observed_digest,
        binary_bytes,
    })
}

fn tool_identity(
    tool: &VerifiedTool,
    args: &[&str],
    repo: &Path,
    environment: &BTreeMap<String, String>,
    distribution_digest: Option<String>,
    invocations: &mut Vec<InvocationReceipt>,
) -> Result<(ToolIdentity, String)> {
    let result = run_command(tool, args, repo, environment)?;
    ensure_command_success(&format!("{} version", tool.tool_id), &result)?;
    let mut bytes = result.output.stdout.clone();
    bytes.extend_from_slice(&result.output.stderr);
    let version = String::from_utf8_lossy(&bytes).trim().to_owned();
    ensure!(
        !version.is_empty(),
        "{} returned an empty version",
        tool.tool_id
    );
    let identity = ToolIdentity {
        tool_id: tool.tool_id.clone(),
        requested_path: tool.requested_path.clone(),
        canonical_path: tool.canonical_path.display().to_string(),
        expected_binary_digest: tool.expected_digest.clone(),
        observed_binary_digest: tool.observed_digest.clone(),
        version: version.clone(),
        version_output_digest: sha256_bytes(&bytes),
        distribution_digest,
    };
    invocations.push(result.receipt);
    Ok((identity, version))
}

fn run_command(
    tool: &VerifiedTool,
    args: &[&str],
    working_directory: &Path,
    environment: &BTreeMap<String, String>,
) -> Result<CommandResult> {
    let started = Instant::now();
    let mut command = Command::new(&tool.canonical_path);
    command.args(args).current_dir(working_directory);
    for (key, value) in environment {
        command.env(key, value);
    }
    let output = command
        .output()
        .with_context(|| format!("execute {}", tool.canonical_path.display()))?;
    let receipt = InvocationReceipt {
        tool_id: tool.tool_id.clone(),
        executable_digest: tool.observed_digest.clone(),
        argv: std::iter::once(tool.canonical_path.display().to_string())
            .chain(args.iter().map(|value| (*value).to_owned()))
            .collect(),
        working_directory: working_directory.display().to_string(),
        environment_digest: environment_digest(environment)?,
        exit_code: output.status.code(),
        success: output.status.success(),
        stdout_digest: sha256_bytes(&output.stdout),
        stdout_bytes: output.stdout.len() as u64,
        stderr_digest: sha256_bytes(&output.stderr),
        stderr_bytes: output.stderr.len() as u64,
        wall_micros: elapsed_micros(started),
    };
    Ok(CommandResult { output, receipt })
}

fn ensure_command_success(label: &str, result: &CommandResult) -> Result<()> {
    if !result.output.status.success() {
        bail!(
            "{label} failed with {:?}; stderr digest {}, {} bytes",
            result.output.status.code(),
            result.receipt.stderr_digest,
            result.receipt.stderr_bytes
        );
    }
    Ok(())
}

fn execution_environment(cargo: &VerifiedTool, rustc: &VerifiedTool) -> BTreeMap<String, String> {
    let mut environment: BTreeMap<String, String> = env::vars().collect();
    environment.insert(
        "CARGO".to_owned(),
        cargo.canonical_path.display().to_string(),
    );
    environment.insert(
        "RUSTC".to_owned(),
        rustc.canonical_path.display().to_string(),
    );
    let mut search_paths: Vec<PathBuf> =
        [cargo.canonical_path.parent(), rustc.canonical_path.parent()]
            .into_iter()
            .flatten()
            .map(Path::to_path_buf)
            .collect();
    search_paths.extend(env::split_paths(&env::var_os("PATH").unwrap_or_default()));
    let path =
        env::join_paths(search_paths).unwrap_or_else(|_| env::var_os("PATH").unwrap_or_default());
    environment.insert("PATH".to_owned(), path.to_string_lossy().into_owned());
    environment
}

fn environment_digest(environment: &BTreeMap<String, String>) -> Result<String> {
    let digests: BTreeMap<_, _> = environment
        .iter()
        .map(|(key, value)| (key, sha256_bytes(value.as_bytes())))
        .collect();
    canonical_digest(&digests)
}

fn relevant_environment() -> Vec<EnvironmentInput> {
    HASH_ENVIRONMENT_NAMES
        .iter()
        .map(|name| EnvironmentInput {
            key: (*name).to_owned(),
            value: env::var(name)
                .map(|value| sha256_bytes(value.as_bytes()))
                .unwrap_or_else(|_| "ABSENT".to_owned()),
        })
        .collect()
}

fn git_optional(
    git: &VerifiedTool,
    args: &[&str],
    repo: &Path,
    environment: &BTreeMap<String, String>,
    invocations: &mut Vec<InvocationReceipt>,
) -> Result<Option<String>> {
    let result = run_command(git, args, repo, environment)?;
    let value = if result.output.status.success() {
        Some(
            String::from_utf8(result.output.stdout.clone())?
                .trim()
                .to_owned(),
        )
    } else {
        None
    };
    invocations.push(result.receipt);
    Ok(value.filter(|value| !value.is_empty()))
}

fn repository_manifest(repo: &Path) -> Result<RepositoryManifest> {
    let mut entries = Vec::new();
    let mut sources = Vec::new();
    let mut config_artifacts = Vec::new();
    let mut repository_bytes = 0;
    let mut source_bytes = 0;
    for entry in WalkDir::new(repo)
        .follow_links(false)
        .sort_by_file_name()
        .into_iter()
        .filter_entry(|entry| !is_repository_exclusion(repo, entry))
    {
        let entry = entry?;
        if entry.path() == repo || entry.file_type().is_dir() {
            continue;
        }
        let relative = normalized_relative(repo, entry.path())?;
        let (kind, digest, bytes) = if entry.file_type().is_symlink() {
            let target = fs::read_link(entry.path())?;
            let target = target.as_os_str().as_encoded_bytes();
            ("symlink", sha256_bytes(target), target.len() as u64)
        } else if entry.file_type().is_file() {
            let (digest, bytes) = hash_file(entry.path())?;
            ("file", digest, bytes)
        } else {
            bail!("unsupported repository entry type: {relative}");
        };
        repository_bytes += bytes;
        entries.push((relative.clone(), kind.to_owned(), digest.clone(), bytes));
        if entry.file_type().is_file() && entry.path().extension() == Some(OsStr::new("rs")) {
            source_bytes += bytes;
            sources.push(source_artifact(&relative, &digest, bytes, "USER"));
        }
        if entry.file_type().is_file() && is_build_configuration_path(&relative) {
            config_artifacts.push(source_artifact(
                &relative,
                &digest,
                bytes,
                "BUILD_CONFIGURATION",
            ));
        }
    }
    entries.sort();
    sources.sort_by(|left, right| left.artifact_id.cmp(&right.artifact_id));
    config_artifacts.sort_by(|left, right| left.normalized_path.cmp(&right.normalized_path));
    Ok(RepositoryManifest {
        tree_digest: canonical_digest(&entries)?,
        repository_bytes,
        source_bytes,
        sources,
        config_artifacts,
    })
}

fn source_artifact(path: &str, digest: &str, bytes: u64, origin: &str) -> SourceArtifact {
    SourceArtifact {
        artifact_id: sha256_bytes(format!("{path}\0{digest}").as_bytes()),
        normalized_path: path.to_owned(),
        content_digest: digest.to_owned(),
        size_bytes: bytes,
        origin: origin.to_owned(),
    }
}

fn is_repository_exclusion(repo: &Path, entry: &DirEntry) -> bool {
    if entry.path() == repo {
        return false;
    }
    matches!(entry.file_name().to_str(), Some(".git" | "target")) && entry.file_type().is_dir()
}

fn is_build_configuration_path(path: &str) -> bool {
    matches!(
        path,
        "Cargo.toml"
            | "Cargo.lock"
            | "rust-toolchain"
            | "rust-toolchain.toml"
            | ".cargo/config"
            | ".cargo/config.toml"
    ) || path.ends_with("/Cargo.toml")
        || path.ends_with("/build.rs")
        || path.ends_with("/.cargo/config")
        || path.ends_with("/.cargo/config.toml")
}

fn normalized_relative(root: &Path, path: &Path) -> Result<String> {
    let relative = path.strip_prefix(root)?;
    ensure!(!relative.is_absolute(), "relative path became absolute");
    let mut parts = Vec::new();
    for component in relative.components() {
        match component {
            Component::Normal(value) => parts.push(
                value
                    .to_str()
                    .ok_or_else(|| anyhow!("non-UTF-8 path under repository"))?,
            ),
            _ => bail!("non-normal repository path component"),
        }
    }
    Ok(parts.join("/"))
}

fn hash_file(path: &Path) -> Result<(String, u64)> {
    let bytes = fs::read(path).with_context(|| format!("read {}", path.display()))?;
    Ok((sha256_bytes(&bytes), bytes.len() as u64))
}

fn hash_directory(root: &Path) -> Result<(String, u64)> {
    let mut hasher = Sha256::new();
    let mut total_bytes = 0;
    for entry in WalkDir::new(root)
        .follow_links(false)
        .sort_by_file_name()
        .into_iter()
    {
        let entry = entry?;
        if entry.path() == root || entry.file_type().is_dir() {
            continue;
        }
        let relative = normalized_relative(root, entry.path())?;
        let (kind, digest, bytes) = if entry.file_type().is_symlink() {
            let target = fs::read_link(entry.path())?;
            let encoded = target.as_os_str().as_encoded_bytes();
            (b'L', sha256_bytes(encoded), encoded.len() as u64)
        } else if entry.file_type().is_file() {
            let (digest, bytes) = hash_file(entry.path())?;
            (b'F', digest, bytes)
        } else {
            continue;
        };
        hasher.update(relative.as_bytes());
        hasher.update([0, kind]);
        hasher.update(digest.as_bytes());
        hasher.update([0]);
        total_bytes += bytes;
    }
    Ok((
        format!("sha256:{}", hex::encode(hasher.finalize())),
        total_bytes,
    ))
}

fn dependency_graph_digest(metadata: &Value) -> Result<String> {
    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct DependencyGraph<'a> {
        packages: &'a Value,
        resolve: &'a Value,
        workspace_members: &'a Value,
        workspace_default_members: &'a Value,
    }
    let object = metadata
        .as_object()
        .ok_or_else(|| anyhow!("cargo metadata root is not an object"))?;
    let value = DependencyGraph {
        packages: object.get("packages").unwrap_or(&Value::Null),
        resolve: object.get("resolve").unwrap_or(&Value::Null),
        workspace_members: object.get("workspace_members").unwrap_or(&Value::Null),
        workspace_default_members: object
            .get("workspace_default_members")
            .unwrap_or(&Value::Null),
    };
    canonical_digest(&value)
}

fn metadata_targets(metadata: &Value, repo: &Path) -> Result<Vec<CargoTarget>> {
    let mut targets = Vec::new();
    for package in metadata
        .get("packages")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("cargo metadata has no packages array"))?
    {
        let package_id = json_string(package, "id")?;
        let package_name = json_string(package, "name")?;
        let edition = json_string(package, "edition")?;
        for target in package
            .get("targets")
            .and_then(Value::as_array)
            .ok_or_else(|| anyhow!("cargo metadata package has no targets"))?
        {
            let source_path = PathBuf::from(json_string(target, "src_path")?);
            let source_path = if source_path.starts_with(repo) {
                normalized_relative(repo, &source_path)?
            } else {
                source_path.display().to_string()
            };
            targets.push(CargoTarget {
                package_id: package_id.to_owned(),
                package_name: package_name.to_owned(),
                target_name: json_string(target, "name")?.to_owned(),
                kinds: json_string_array(target, "kind")?,
                crate_types: json_string_array(target, "crate_types")?,
                source_path,
                edition: edition.to_owned(),
                required_features: json_string_array(target, "required-features")
                    .or_else(|_| json_string_array(target, "required_features"))
                    .unwrap_or_default(),
            });
        }
    }
    targets.sort_by(|left, right| {
        (&left.package_id, &left.target_name).cmp(&(&right.package_id, &right.target_name))
    });
    Ok(targets)
}

fn enabled_features(metadata: &Value) -> Result<Vec<String>> {
    let mut features = BTreeSet::new();
    if let Some(nodes) = metadata
        .get("resolve")
        .and_then(|value| value.get("nodes"))
        .and_then(Value::as_array)
    {
        for node in nodes {
            let package_id = json_string(node, "id")?;
            for feature in json_string_array(node, "features")? {
                features.insert(format!("{package_id}#{feature}"));
            }
        }
    }
    Ok(features.into_iter().collect())
}

fn configured_platform(rustc_version: &str) -> String {
    rustc_version
        .lines()
        .find_map(|line| line.strip_prefix("host: "))
        .unwrap_or("unknown-rustc-host")
        .to_owned()
}

fn first_line(value: &str) -> &str {
    value.lines().next().unwrap_or(value)
}

fn cargo_diagnostics(stdout: &[u8]) -> Result<(String, u64)> {
    let mut diagnostics = Vec::new();
    for line in stdout.split(|byte| *byte == b'\n') {
        if line.is_empty() {
            continue;
        }
        let value: Value = serde_json::from_slice(line).context("parse cargo check JSON line")?;
        if value.get("reason").and_then(Value::as_str) != Some("compiler-message") {
            continue;
        }
        let message = value
            .get("message")
            .and_then(Value::as_object)
            .ok_or_else(|| anyhow!("cargo compiler-message has no message object"))?;
        let spans = message
            .get("spans")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .map(|span| {
                serde_json::json!({
                    "fileName": span.get("file_name"),
                    "byteStart": span.get("byte_start"),
                    "byteEnd": span.get("byte_end"),
                    "isPrimary": span.get("is_primary"),
                })
            })
            .collect::<Vec<_>>();
        diagnostics.push(serde_json::json!({
            "packageId": value.get("package_id"),
            "targetName": value.get("target").and_then(|target| target.get("name")),
            "level": message.get("level"),
            "code": message.get("code").and_then(|code| code.get("code")),
            "message": message.get("message"),
            "spans": spans,
        }));
    }
    diagnostics.sort_by_key(|value| canonical_json(value).unwrap_or_default());
    Ok((canonical_digest(&diagnostics)?, diagnostics.len() as u64))
}

fn json_string<'a>(value: &'a Value, name: &str) -> Result<&'a str> {
    value
        .get(name)
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("missing string field {name}"))
}

fn json_string_array(value: &Value, name: &str) -> Result<Vec<String>> {
    value
        .get(name)
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("missing array field {name}"))?
        .iter()
        .map(|item| {
            item.as_str()
                .map(ToOwned::to_owned)
                .ok_or_else(|| anyhow!("non-string value in {name}"))
        })
        .collect::<Result<Vec<_>>>()
}

fn build_configuration_digest(
    artifacts: &[SourceArtifact],
    environment: &[EnvironmentInput],
    inherited_environment_digest: &str,
) -> Result<String> {
    canonical_digest(&serde_json::json!({
        "cargoMetadataArgs": ["--format-version", "1", "--locked"],
        "cargoCheckArgs": ["--workspace", "--all-targets", "--locked", "--message-format=json"],
        "rustAnalyzerScipArgs": ["scip", "<absolute-repo>", "--output", "<temporary-output>"],
        "configurationArtifacts": artifacts,
        "relevantEnvironment": environment,
        "inheritedEnvironmentDigest": inherited_environment_digest,
    }))
}

fn parse_scip(
    index: &ScipIndex,
    repo: &Path,
    sources: &[SourceArtifact],
    evidence_id: &str,
) -> Result<ParsedScip> {
    let source_evidence = load_source_evidence(repo, sources)?;
    let mut metadata = BTreeMap::<String, EntityMetadata>::new();
    for symbol in &index.external_symbols {
        metadata.insert(symbol.symbol.clone(), entity_metadata(symbol, "external"));
    }
    for document in &index.documents {
        let path = normalize_scip_path(repo, &document.relative_path)?;
        for symbol in &document.symbols {
            metadata.insert(
                scoped_native_identity(&path, &symbol.symbol),
                entity_metadata(symbol, "workspace"),
            );
        }
    }

    let mut parsed_occurrences = Vec::<ParsedOccurrence>::new();
    let mut definition_ranges = BTreeMap::<String, BTreeSet<EvidenceRange>>::new();
    let mut definition_locations = BTreeMap::<String, BTreeSet<Location>>::new();
    let mut all_identities = BTreeSet::<String>::new();

    for document in &index.documents {
        let path = normalize_scip_path(repo, &document.relative_path)?;
        let document_native = document_native_identity(&path);
        all_identities.insert(document_native.clone());
        metadata.insert(
            document_native,
            EntityMetadata {
                display_name: Some(path.clone()),
                kind_hint: "MODULE".to_owned(),
                language_payload: BTreeMap::from([
                    ("source".to_owned(), "scip-document".to_owned()),
                    ("language".to_owned(), document.language.clone()),
                ]),
            },
        );
        ensure!(
            document.language.is_empty() || document.language.eq_ignore_ascii_case("rust"),
            "SCIP document {} unexpectedly declares language {}",
            path,
            document.language
        );
        let position_encoding = document.position_encoding.enum_value_or_default();
        ensure!(
            position_encoding == scip_types::PositionEncoding::UTF8CodeUnitOffsetFromLineStart,
            "SCIP document {} uses unsupported position encoding {:?}; exact byte ranges require UTF-8 code-unit offsets",
            path,
            position_encoding
        );
        let encoding = format!("{position_encoding:?}");
        for occurrence in &document.occurrences {
            if occurrence.symbol.is_empty() {
                continue;
            }
            let native_identity = scoped_native_identity(&path, &occurrence.symbol);
            let entity_id = entity_id(&native_identity);
            all_identities.insert(native_identity.clone());
            let range = scip_range(occurrence)?;
            let evidence_range = exact_evidence_range(&path, &range, &source_evidence)?;
            let source_location = Location {
                path: path.clone(),
                range: range.clone(),
            };
            let roles = occurrence_roles(occurrence.symbol_roles);
            for role in roles {
                let id_material = (&native_identity, &evidence_range, role);
                let output = Occurrence {
                    occurrence_id: canonical_digest(&id_material)?,
                    role: role.to_owned(),
                    origin: if occurrence.symbol_roles & 16 != 0 {
                        "GENERATED"
                    } else {
                        "SOURCE"
                    }
                    .to_owned(),
                    grade: "COMPILER_RESOLVED".to_owned(),
                    entity_id: entity_id.clone(),
                    range: evidence_range.clone(),
                    source_location: source_location.clone(),
                    native_identity: occurrence.symbol.clone(),
                    position_encoding: encoding.clone(),
                };
                if role == "DEFINITION" || role == "DECLARATION" {
                    definition_ranges
                        .entry(native_identity.clone())
                        .or_default()
                        .insert(evidence_range.clone());
                    definition_locations
                        .entry(native_identity.clone())
                        .or_default()
                        .insert(source_location.clone());
                }
                parsed_occurrences.push(ParsedOccurrence { output });
            }
        }
    }

    let mut facts = Vec::<Fact>::new();
    for document in &index.documents {
        let path = normalize_scip_path(repo, &document.relative_path)?;
        for symbol in &document.symbols {
            let owner_native = scoped_native_identity(&path, &symbol.symbol);
            let owner = entity_id(&owner_native);
            all_identities.insert(owner_native);
            for relationship in &symbol.relationships {
                if relationship.symbol.is_empty() {
                    continue;
                }
                let target_native = scoped_native_identity(&path, &relationship.symbol);
                let target = entity_id(&target_native);
                all_identities.insert(target_native);
                let mut kinds = Vec::new();
                if relationship.is_reference {
                    kinds.push("REFERENCE");
                }
                if relationship.is_implementation {
                    kinds.push("IMPLEMENTATION");
                }
                if relationship.is_type_definition {
                    kinds.push("TYPE_DEFINITION");
                }
                if relationship.is_definition {
                    kinds.push("DEFINITION");
                }
                if kinds.is_empty() {
                    kinds.push("UNSPECIFIED_SCIP_RELATIONSHIP");
                }
                facts.push(make_fact(
                    RELATIONSHIP_OPERATION,
                    &owner,
                    &target,
                    kinds.into_iter().map(ToOwned::to_owned).collect(),
                    None,
                    None,
                    evidence_id,
                )?);
            }
        }
    }

    for occurrence in &parsed_occurrences {
        if matches!(
            occurrence.output.role.as_str(),
            "DEFINITION" | "DECLARATION"
        ) {
            continue;
        }
        let document_owner = entity_id(&document_native_identity(
            &occurrence.output.source_location.path,
        ));
        facts.push(make_fact(
            DOCUMENT_OCCURRENCE_OPERATION,
            &document_owner,
            &occurrence.output.entity_id,
            vec![format!("RESOLVED_{}", occurrence.output.role)],
            Some(occurrence.output.range.clone()),
            Some(occurrence.output.source_location.clone()),
            evidence_id,
        )?);
    }

    let mut entities = Vec::new();
    for native_identity in all_identities {
        let scoped_metadata = metadata
            .get(&native_identity)
            .or_else(|| metadata.get(unscoped_identity(&native_identity)));
        let definitions: Vec<_> = definition_ranges
            .get(&native_identity)
            .into_iter()
            .flat_map(|locations| locations.iter().cloned())
            .collect();
        let source_definitions: Vec<_> = definition_locations
            .get(&native_identity)
            .into_iter()
            .flat_map(|locations| locations.iter().cloned())
            .collect();
        let document_scope = document_scope(&native_identity);
        entities.push(Entity {
            adapter_namespace: format!("{ADAPTER_ID}/{ADAPTER_VERSION}"),
            opaque_id: entity_id(&native_identity),
            resolution: if native_identity.starts_with("document@") {
                "SYNTHETIC"
            } else {
                "RESOLVED"
            }
            .to_owned(),
            coarse_kind: scoped_metadata
                .map(|item| item.kind_hint.clone())
                .unwrap_or_else(|| "VALUE_LIKE".to_owned()),
            display_name: scoped_metadata.and_then(|item| item.display_name.clone()),
            primary_definition: definitions.first().cloned(),
            language_payload: scoped_metadata
                .map(|item| item.language_payload.clone())
                .unwrap_or_else(|| {
                    BTreeMap::from([("source".to_owned(), "scip-occurrence".to_owned())])
                }),
            native_identity: unscoped_identity(&native_identity).to_owned(),
            document_scope,
            definition_locations: source_definitions,
        });
    }

    let mut occurrences: Vec<_> = parsed_occurrences
        .into_iter()
        .map(|item| item.output)
        .collect();
    entities.sort_by(|left, right| left.opaque_id.cmp(&right.opaque_id));
    occurrences.sort_by(|left, right| left.occurrence_id.cmp(&right.occurrence_id));
    facts.sort_by(|left, right| left.fact_id.cmp(&right.fact_id));
    facts.dedup_by(|left, right| left.fact_id == right.fact_id);
    Ok(ParsedScip {
        entities,
        occurrences,
        facts,
    })
}

fn load_source_evidence(
    repo: &Path,
    sources: &[SourceArtifact],
) -> Result<BTreeMap<String, SourceEvidence>> {
    let mut evidence = BTreeMap::new();
    for source in sources {
        let bytes = fs::read(repo.join(&source.normalized_path))
            .with_context(|| format!("read source {}", source.normalized_path))?;
        ensure!(
            bytes.len() as u64 == source.size_bytes,
            "source size changed before SCIP range conversion: {}",
            source.normalized_path
        );
        ensure!(
            sha256_bytes(&bytes) == source.content_digest,
            "source digest changed before SCIP range conversion: {}",
            source.normalized_path
        );
        std::str::from_utf8(&bytes)
            .with_context(|| format!("Rust source {} is not UTF-8", source.normalized_path))?;
        let mut line_starts = vec![0_usize];
        for (index, byte) in bytes.iter().enumerate() {
            if *byte == b'\n' {
                line_starts.push(index + 1);
            }
        }
        evidence.insert(
            source.normalized_path.clone(),
            SourceEvidence {
                artifact_id: source.artifact_id.clone(),
                content_digest: source.content_digest.clone(),
                bytes,
                line_starts,
            },
        );
    }
    Ok(evidence)
}

fn exact_evidence_range(
    path: &str,
    range: &SourceRange,
    sources: &BTreeMap<String, SourceEvidence>,
) -> Result<EvidenceRange> {
    let source = sources.get(path).with_context(|| {
        format!("SCIP document {path} is absent from the captured Rust sources")
    })?;
    let start_byte = utf8_byte_offset(source, range.start_line, range.start_character, path)?;
    let end_byte = utf8_byte_offset(source, range.end_line, range.end_character, path)?;
    ensure!(
        start_byte <= end_byte,
        "SCIP range for {path} converts to reversed byte offsets"
    );
    Ok(EvidenceRange {
        artifact_id: source.artifact_id.clone(),
        artifact_content_digest: source.content_digest.clone(),
        start_byte,
        end_byte,
    })
}

fn utf8_byte_offset(source: &SourceEvidence, line: u32, character: u32, path: &str) -> Result<u64> {
    let start = *source
        .line_starts
        .get(line as usize)
        .with_context(|| format!("SCIP line {line} is outside {path}"))?;
    let next = source
        .line_starts
        .get(line as usize + 1)
        .copied()
        .unwrap_or(source.bytes.len());
    let mut content_end = next;
    if content_end > start && source.bytes[content_end - 1] == b'\n' {
        content_end -= 1;
    }
    if content_end > start && source.bytes[content_end - 1] == b'\r' {
        content_end -= 1;
    }
    let offset = character as usize;
    ensure!(
        offset <= content_end - start,
        "SCIP UTF-8 character offset {character} exceeds line {line} in {path}"
    );
    let absolute = start + offset;
    ensure!(
        absolute == source.bytes.len() || source.bytes[absolute] & 0b1100_0000 != 0b1000_0000,
        "SCIP UTF-8 offset {character} is not a code-point boundary on line {line} in {path}"
    );
    Ok(absolute as u64)
}

fn entity_metadata(symbol: &scip_types::SymbolInformation, source: &str) -> EntityMetadata {
    let mut language_payload = BTreeMap::new();
    language_payload.insert("source".to_owned(), source.to_owned());
    language_payload.insert(
        "scipKind".to_owned(),
        format!("{:?}", symbol.kind.enum_value_or_default()),
    );
    language_payload.insert(
        "relationshipCount".to_owned(),
        symbol.relationships.len().to_string(),
    );
    EntityMetadata {
        display_name: (!symbol.display_name.is_empty()).then(|| symbol.display_name.clone()),
        kind_hint: coarse_kind_for_scip(symbol.kind.enum_value_or_default()).to_owned(),
        language_payload,
    }
}

fn coarse_kind_for_scip(kind: scip_types::symbol_information::Kind) -> &'static str {
    use scip_types::symbol_information::Kind;
    match kind {
        Kind::Module | Kind::File | Kind::Package | Kind::PackageObject | Kind::Library => "MODULE",
        Kind::Namespace => "NAMESPACE",
        Kind::Class
        | Kind::Concept
        | Kind::Enum
        | Kind::Interface
        | Kind::Object
        | Kind::Protocol
        | Kind::Struct
        | Kind::Trait
        | Kind::Type
        | Kind::TypeAlias
        | Kind::TypeClass => "TYPE_LIKE",
        Kind::AbstractMethod
        | Kind::Accessor
        | Kind::Constructor
        | Kind::Function
        | Kind::Getter
        | Kind::Method
        | Kind::MethodAlias
        | Kind::MethodSpecification
        | Kind::Operator
        | Kind::Predicate
        | Kind::ProtocolMethod
        | Kind::PureVirtualMethod
        | Kind::Setter
        | Kind::SingletonMethod
        | Kind::StaticMethod
        | Kind::TraitMethod
        | Kind::TypeClassMethod => "CALLABLE",
        Kind::Field | Kind::StaticField | Kind::StaticDataMember => "FIELD_LIKE",
        Kind::Macro | Kind::Quasiquoter => "MACRO_LIKE",
        _ => "VALUE_LIKE",
    }
}

fn make_fact(
    relation: &str,
    owner: &str,
    target: &str,
    relation_kinds: Vec<String>,
    range: Option<EvidenceRange>,
    source_location: Option<Location>,
    evidence_id: &str,
) -> Result<Fact> {
    let fact_id = canonical_digest(&(
        relation,
        OPERATION_VERSION,
        owner,
        target,
        &relation_kinds,
        &range,
        evidence_id,
    ))?;
    Ok(Fact {
        fact_id,
        relation: relation.to_owned(),
        owner: owner.to_owned(),
        target: target.to_owned(),
        truth: "TRUE".to_owned(),
        grade: "COMPILER_RESOLVED".to_owned(),
        enumeration: "PARTIAL".to_owned(),
        range,
        provider_payload: FactProviderPayload {
            operation_version: OPERATION_VERSION.to_owned(),
            approximation: "EXACT_ASSERTION_PARTIAL_ENUMERATION".to_owned(),
            relation_kinds,
            evidence_ids: vec![evidence_id.to_owned()],
            source_location,
        },
    })
}

fn normalize_scip_path(repo: &Path, path: &str) -> Result<String> {
    let path = Path::new(path);
    let normalized = if path.is_absolute() {
        normalized_relative(repo, path).context("SCIP document outside repository")?
    } else {
        let joined = repo.join(path);
        normalized_relative(repo, &joined)?
    };
    ensure!(!normalized.is_empty(), "SCIP document path is empty");
    Ok(normalized)
}

fn scoped_native_identity(path: &str, symbol: &str) -> String {
    if symbol.starts_with("local ") {
        format!("local@{path}\0{symbol}")
    } else {
        symbol.to_owned()
    }
}

fn unscoped_identity(identity: &str) -> &str {
    identity
        .split_once('\0')
        .map_or(identity, |(_, symbol)| symbol)
}

fn document_scope(identity: &str) -> Option<String> {
    if let Some(path) = identity.strip_prefix("document@") {
        return Some(path.to_owned());
    }
    identity
        .strip_prefix("local@")
        .and_then(|value| value.split_once('\0'))
        .map(|(path, _)| path.to_owned())
}

fn document_native_identity(path: &str) -> String {
    format!("document@{path}")
}

fn entity_id(native_identity: &str) -> String {
    sha256_bytes(format!("{ADAPTER_ID}\0{ADAPTER_VERSION}\0{native_identity}").as_bytes())
}

fn occurrence_roles(bits: i32) -> Vec<&'static str> {
    let mut roles = Vec::new();
    if bits & 1 != 0 {
        roles.push("DEFINITION");
    }
    if bits & 64 != 0 {
        roles.push("DECLARATION");
    }
    if bits & 2 != 0 {
        roles.push("IMPORT");
    }
    if bits & 4 != 0 {
        roles.push("WRITE");
    }
    if bits & 8 != 0 {
        roles.push("READ");
    }
    if roles.is_empty() {
        roles.push("REFERENCE");
    }
    roles
}

fn scip_range(occurrence: &scip_types::Occurrence) -> Result<SourceRange> {
    match &occurrence.typed_range {
        Some(occurrence::Typed_range::SingleLineRange(range)) => nonnegative_range(
            range.line,
            range.start_character,
            range.line,
            range.end_character,
        ),
        Some(occurrence::Typed_range::MultiLineRange(range)) => nonnegative_range(
            range.start_line,
            range.start_character,
            range.end_line,
            range.end_character,
        ),
        Some(_) => bail!("unsupported future SCIP typed range variant"),
        None => legacy_range(&occurrence.range),
    }
}

fn legacy_range(range: &[i32]) -> Result<SourceRange> {
    match range {
        [line, start, end] => nonnegative_range(*line, *start, *line, *end),
        [start_line, start, end_line, end] => {
            nonnegative_range(*start_line, *start, *end_line, *end)
        }
        _ => bail!("invalid SCIP legacy range with {} elements", range.len()),
    }
}

fn nonnegative_range(
    start_line: i32,
    start_character: i32,
    end_line: i32,
    end_character: i32,
) -> Result<SourceRange> {
    ensure!(
        start_line >= 0 && start_character >= 0 && end_line >= 0 && end_character >= 0,
        "negative SCIP source coordinate"
    );
    ensure!(
        (start_line, start_character) <= (end_line, end_character),
        "reversed SCIP source range"
    );
    Ok(SourceRange {
        start_line: start_line as u32,
        start_character: start_character as u32,
        end_line: end_line as u32,
        end_character: end_character as u32,
    })
}

fn rust_boundaries(no_vcs_revision: bool) -> Vec<Boundary> {
    let mut specifications = vec![
        (
            "macro-expansion",
            "urn:codeclew:boundary:rust:macro-expansion",
            "ENUMERATION_INCOMPLETE",
            "Declarative macro expansion can synthesize symbols and edges whose source provenance is not guaranteed complete by the SCIP producer.",
        ),
        (
            "procedural-macro",
            "urn:codeclew:boundary:rust:procedural-macro",
            "PROOF_INVALID",
            "Procedural macros execute trusted repository/dependency code; emitted SCIP navigation is not a proof of macro behaviour.",
        ),
        (
            "conditional-compilation",
            "urn:codeclew:boundary:rust:cfg",
            "ENUMERATION_INCOMPLETE",
            "Only the captured target, features and environment configuration is analyzed; inactive cfg branches are outside enumeration.",
        ),
        (
            "dynamic-dispatch",
            "urn:codeclew:boundary:rust:dynamic-dispatch",
            "ENUMERATION_INCOMPLETE",
            "Trait objects, function pointers and runtime registration can introduce targets not enumerated by resolved source occurrences.",
        ),
        (
            "unsafe-rust",
            "urn:codeclew:boundary:rust:unsafe",
            "PROOF_INVALID",
            "Navigation evidence and cargo check do not prove aliasing, memory-safety or unsafe-code invariants.",
        ),
        (
            "foreign-function-interface",
            "urn:codeclew:boundary:rust:ffi",
            "ENUMERATION_INCOMPLETE",
            "Native symbols, link-time resolution and callbacks across FFI are outside the Rust SCIP graph.",
        ),
        (
            "build-script",
            "urn:codeclew:boundary:rust:build-script",
            "ENUMERATION_INCOMPLETE",
            "Cargo build scripts may generate source, cfg flags, links and environment values; generated-source enumeration is UNKNOWN.",
        ),
        (
            "generated-sources",
            "urn:codeclew:boundary:rust:generated-sources",
            "ENUMERATION_INCOMPLETE",
            "No provider contract establishes a complete generated-source manifest.",
        ),
        (
            "reflection-and-dynamic-loading",
            "urn:codeclew:boundary:rust:dynamic-loading",
            "ENUMERATION_INCOMPLETE",
            "Symbol lookup by strings, plugin loading and runtime registries are not statically enumerated.",
        ),
    ];
    if no_vcs_revision {
        specifications.push((
            "vcs-revision",
            "urn:codeclew:boundary:snapshot:no-vcs-revision",
            "LOCAL_ONLY",
            "Repository is not bound to a Git revision; the content tree digest remains authoritative.",
        ));
    }
    specifications
        .into_iter()
        .map(|(category, kind_uri, consequence, explanation)| Boundary {
            boundary_id: sha256_bytes(
                format!("{ADAPTER_ID}\0{ADAPTER_VERSION}\0{kind_uri}").as_bytes(),
            ),
            kind_uri: kind_uri.to_owned(),
            consequence: consequence.to_owned(),
            origin: "ADAPTER_DECLARATION".to_owned(),
            provider: ADAPTER_ID.to_owned(),
            details: BoundaryDetails {
                status: "OPEN".to_owned(),
                category: category.to_owned(),
                explanation: explanation.to_owned(),
            },
        })
        .collect()
}

fn capabilities(
    configuration_digest: &str,
    toolchain_digest: &str,
    target_digest: &str,
    known_boundary_kinds: &[String],
) -> Vec<CapabilityDescriptor> {
    [
        (
            NAVIGATION_OPERATION,
            "COMPILER_RESOLVED",
            "EXACT",
            "COLD_INDEX",
            "exact resolved SCIP occurrences emitted by the pinned rust-analyzer; enumeration remains partial",
        ),
        (
            RELATIONSHIP_OPERATION,
            "COMPILER_RESOLVED",
            "EXACT",
            "COLD_INDEX",
            "exact SCIP symbol relationships emitted by the pinned rust-analyzer; enumeration remains partial",
        ),
        (
            DOCUMENT_OCCURRENCE_OPERATION,
            "COMPILER_RESOLVED",
            "EXACT",
            "IN_MEMORY_GRAPH_CONSTRUCTION",
            "exact document membership for each captured resolved SCIP occurrence; enumeration remains partial",
        ),
        (
            CHECK_OPERATION,
            "COMPILER_CHECKED",
            "NOT_APPLICABLE",
            "COMPILER_INVOCATION",
            "acceptance receipt for cargo check --workspace --all-targets --locked under the captured snapshot",
        ),
        (
            IMPACT_OPERATION,
            "STATICALLY_APPROXIMATED",
            "HEURISTIC",
            "IN_MEMORY_GRAPH_QUERY",
            "bounded reverse closure over resolved adapter facts with all declared boundaries retained",
        ),
    ]
    .into_iter()
    .map(
        |(operation_uri, grade, approximation, cost_class, semantics)| CapabilityDescriptor {
            operation_uri: operation_uri.to_owned(),
            language_id: "rust".to_owned(),
            adapter_id: ADAPTER_ID.to_owned(),
            adapter_version: ADAPTER_VERSION.to_owned(),
            grade: grade.to_owned(),
            support: "SUPPORTED".to_owned(),
            guaranteed_enumeration: "PARTIAL".to_owned(),
            operation_version: OPERATION_VERSION.to_owned(),
            operation_specification_digest: operation_specification_digest(
                operation_uri,
                semantics,
            ),
            toolchain_digest: toolchain_digest.to_owned(),
            build_configuration_digest: configuration_digest.to_owned(),
            target_digest: target_digest.to_owned(),
            approximation: approximation.to_owned(),
            known_boundary_kinds: known_boundary_kinds.to_vec(),
            cost_class: cost_class.to_owned(),
        },
    )
    .collect()
}

fn operation_specification_digest(operation_uri: &str, semantics: &str) -> String {
    sha256_bytes(format!("{operation_uri}\0{OPERATION_VERSION}\0{semantics}").as_bytes())
}

fn may_impact(
    entities: &[Entity],
    occurrences: &[Occurrence],
    facts: &[Fact],
    boundaries: &[Boundary],
    query: ImpactQuery<'_>,
) -> Result<ImpactOutput> {
    let started = Instant::now();
    let max_depth = query.max_depth;
    let max_entities = query.max_entities;
    let base = |status: &str, reason: &str, seed_entity: Option<String>| ImpactOutput {
        schema: crate::protocol::IMPACT_SCHEMA.to_owned(),
        status: status.to_owned(),
        reason: reason.to_owned(),
        closure_specification: "codeclew.impact.reverse-resolved-relations/0.1".to_owned(),
        seed_entity,
        max_depth,
        max_entities,
        affected: Vec::new(),
        paths: Vec::new(),
        mandatory_obligations: boundary_obligations(boundaries),
        boundaries: boundaries.to_vec(),
        query_micros: elapsed_micros(started),
    };
    let Some(requested_seed) = query.requested_seed else {
        return Ok(base("UNKNOWN", "NO_SEED_ENTITY", None));
    };

    let mut matching_ids: Vec<_> = entities
        .iter()
        .filter(|entity| {
            entity.opaque_id == requested_seed || entity.native_identity == requested_seed
        })
        .map(|entity| entity.opaque_id.clone())
        .collect();
    matching_ids.sort();
    matching_ids.dedup();
    if matching_ids.is_empty() {
        let mut output = base("UNKNOWN", "UNRESOLVED_SEED_ENTITY", None);
        output.mandatory_obligations.push(MandatoryObligation {
            id: "resolve-seed".to_owned(),
            kind: "codeclew.obligation/resolve-entity/1".to_owned(),
            mandatory: true,
            status: "UNKNOWN".to_owned(),
            boundary_digest: None,
        });
        return Ok(output);
    }
    if matching_ids.len() != 1 {
        let mut output = base("UNKNOWN", "AMBIGUOUS_SEED_ENTITY", None);
        output.mandatory_obligations.push(MandatoryObligation {
            id: "disambiguate-seed".to_owned(),
            kind: "codeclew.obligation/disambiguate-entity/1".to_owned(),
            mandatory: true,
            status: "UNKNOWN".to_owned(),
            boundary_digest: None,
        });
        return Ok(output);
    }
    let seed = matching_ids.remove(0);
    let documents = entity_documents(occurrences, facts);
    let mut output = base(
        "PARTIAL_BOUNDARY",
        "OPEN_LANGUAGE_BOUNDARIES",
        Some(seed.clone()),
    );
    output.affected.push(ImpactAffected {
        entity_id: seed.clone(),
        impact_class: "DEFINITE".to_owned(),
        depth: 0,
        documents: documents.get(&seed).cloned().unwrap_or_default(),
    });
    let mut visited = BTreeSet::from([seed.clone()]);
    let mut queue = VecDeque::from([(seed, 0_u32)]);
    let mut budget_truncated = false;
    while let Some((target, depth)) = queue.pop_front() {
        let inbound: Vec<_> = facts
            .iter()
            .filter(|fact| fact.truth == "TRUE" && fact.target == target)
            .collect();
        if depth >= max_depth {
            budget_truncated |= inbound.iter().any(|fact| !visited.contains(&fact.owner));
            continue;
        }
        for fact in inbound {
            output.paths.push(ImpactPath {
                from: target.clone(),
                to: fact.owner.clone(),
                fact_id: fact.fact_id.clone(),
                relation: fact.relation.clone(),
            });
            if visited.contains(&fact.owner) {
                continue;
            }
            if visited.len() >= max_entities as usize {
                budget_truncated = true;
                break;
            }
            visited.insert(fact.owner.clone());
            output.affected.push(ImpactAffected {
                entity_id: fact.owner.clone(),
                impact_class: "POSSIBLE".to_owned(),
                depth: depth + 1,
                documents: documents.get(&fact.owner).cloned().unwrap_or_default(),
            });
            queue.push_back((fact.owner.clone(), depth + 1));
        }
    }
    output
        .affected
        .sort_by(|left, right| (left.depth, &left.entity_id).cmp(&(right.depth, &right.entity_id)));
    output.paths.sort_by(|left, right| {
        (&left.from, &left.to, &left.fact_id).cmp(&(&right.from, &right.to, &right.fact_id))
    });
    if budget_truncated {
        output.status = "PARTIAL_BUDGET".to_owned();
        output.reason = "QUERY_BUDGET_REACHED".to_owned();
        output.mandatory_obligations.push(MandatoryObligation {
            id: "expand-impact-budget".to_owned(),
            kind: "codeclew.obligation/expand-query-budget/1".to_owned(),
            mandatory: true,
            status: "UNKNOWN".to_owned(),
            boundary_digest: None,
        });
    }
    output.query_micros = elapsed_micros(started);
    Ok(output)
}

fn entity_documents(occurrences: &[Occurrence], facts: &[Fact]) -> BTreeMap<String, Vec<String>> {
    let mut documents = BTreeMap::<String, BTreeSet<String>>::new();
    for occurrence in occurrences {
        documents
            .entry(occurrence.entity_id.clone())
            .or_default()
            .insert(occurrence.source_location.path.clone());
    }
    for fact in facts {
        if let Some(location) = &fact.provider_payload.source_location {
            documents
                .entry(fact.owner.clone())
                .or_default()
                .insert(location.path.clone());
        }
    }
    documents
        .into_iter()
        .map(|(key, value)| (key, value.into_iter().collect()))
        .collect()
}

fn boundary_obligations(boundaries: &[Boundary]) -> Vec<MandatoryObligation> {
    boundaries
        .iter()
        .enumerate()
        .map(|(index, boundary)| MandatoryObligation {
            id: format!("validate-boundary-{index}"),
            kind: "codeclew.obligation/validate-boundary/1".to_owned(),
            mandatory: true,
            status: "UNKNOWN".to_owned(),
            boundary_digest: Some(boundary.boundary_id.clone()),
        })
        .collect()
}

fn seal_output(output: &mut AdapterOutput) -> Result<()> {
    output.output_digest = format!("sha256:{}", "0".repeat(64));
    loop {
        let bytes = canonical_json(output)?.len() as u64;
        if output.cost.emitted_bytes == bytes {
            break;
        }
        output.cost.emitted_bytes = bytes;
    }
    output.output_digest.clear();
    output.output_digest = canonical_digest(output)?;
    Ok(())
}

fn validate_output(
    output: &AdapterOutput,
    source_manifest_digest: &str,
    rust_analyzer_version: &str,
) -> Result<()> {
    ensure!(output.schema == ADAPTER_SCHEMA, "wrong output schema");
    verify_output_digest(output)?;
    ensure!(
        output.cost.emitted_bytes == canonical_json(output)?.len() as u64,
        "emittedBytes does not match the canonical output size"
    );
    ensure!(
        canonical_digest(&output.snapshot_input.sources)? == source_manifest_digest,
        "source manifest changed while assembling output"
    );
    ensure!(
        output.compiler_receipt.snapshot_tree_digest
            == output.snapshot_input.repository_tree_digest,
        "compiler receipt is not bound to the output snapshot"
    );
    ensure!(
        output
            .compiler_receipt
            .provider_payload
            .source_tree_digest_before
            == output
                .compiler_receipt
                .provider_payload
                .source_tree_digest_after,
        "compiler receipt spans different source trees"
    );
    ensure!(
        output
            .snapshot_input
            .toolchain
            .provider_payload
            .tools
            .iter()
            .any(|tool| tool.tool_id == "rust-analyzer" && tool.version == rust_analyzer_version),
        "rust-analyzer provenance missing"
    );
    ensure!(
        output
            .snapshot_input
            .sources
            .windows(2)
            .all(|pair| pair[0].artifact_id < pair[1].artifact_id),
        "source artifacts are not strictly sorted by artifactId"
    );
    let source_ranges: BTreeMap<_, _> = output
        .snapshot_input
        .sources
        .iter()
        .map(|source| {
            (
                source.artifact_id.as_str(),
                (source.content_digest.as_str(), source.size_bytes),
            )
        })
        .collect();
    let validate_range = |range: &EvidenceRange| -> Result<()> {
        let (content_digest, size_bytes) = source_ranges
            .get(range.artifact_id.as_str())
            .context("evidence range references absent source artifact")?;
        ensure!(
            range.artifact_content_digest == *content_digest,
            "evidence range source digest mismatch"
        );
        ensure!(
            range.start_byte <= range.end_byte && range.end_byte <= *size_bytes,
            "evidence range is outside the source artifact"
        );
        Ok(())
    };
    let entity_ids: BTreeSet<_> = output
        .entities
        .iter()
        .map(|entity| entity.opaque_id.as_str())
        .collect();
    for entity in &output.entities {
        ensure!(
            matches!(
                entity.coarse_kind.as_str(),
                "MODULE"
                    | "NAMESPACE"
                    | "TYPE_LIKE"
                    | "CALLABLE"
                    | "VALUE_LIKE"
                    | "FIELD_LIKE"
                    | "MACRO_LIKE"
            ),
            "entity has an unsupported coarse kind"
        );
        if let Some(range) = &entity.primary_definition {
            validate_range(range)?;
        }
    }
    for occurrence in &output.occurrences {
        ensure!(
            entity_ids.contains(occurrence.entity_id.as_str()),
            "occurrence references absent entity"
        );
        ensure!(
            occurrence.grade == "COMPILER_RESOLVED",
            "SCIP occurrence was overgraded"
        );
        validate_range(&occurrence.range)?;
    }
    let capability_grades: BTreeMap<_, _> = output
        .capability_descriptors
        .iter()
        .map(|capability| (capability.operation_uri.as_str(), capability.grade.as_str()))
        .collect();
    for fact in &output.facts {
        ensure!(
            entity_ids.contains(fact.owner.as_str()) && entity_ids.contains(fact.target.as_str()),
            "fact references absent entity"
        );
        ensure!(
            fact.grade == "COMPILER_RESOLVED",
            "SCIP fact was overgraded"
        );
        ensure!(
            fact.enumeration == "PARTIAL",
            "SCIP enumeration was overstated"
        );
        ensure!(
            capability_grades.get(fact.relation.as_str()).copied() == Some(fact.grade.as_str()),
            "fact relation and grade are not bound to a capability"
        );
        if let Some(range) = &fact.range {
            validate_range(range)?;
        }
    }
    ensure!(
        output.impact.status != "COMPLETE_IN_SCOPE" || output.impact.boundaries.is_empty(),
        "impact cannot claim complete coverage with open boundaries"
    );
    Ok(())
}

fn normalize_expected_digest(value: &str) -> Result<String> {
    let hex = value.strip_prefix("sha256:").unwrap_or(value);
    ensure!(
        hex.len() == 64 && hex.bytes().all(|byte| byte.is_ascii_hexdigit()),
        "expected SHA-256 must contain exactly 64 hexadecimal characters"
    );
    Ok(format!("sha256:{}", hex.to_ascii_lowercase()))
}

fn elapsed_micros(started: Instant) -> u64 {
    started.elapsed().as_micros().try_into().unwrap_or(u64::MAX)
}
