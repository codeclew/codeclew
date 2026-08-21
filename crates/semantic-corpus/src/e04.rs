//! Post-freeze E04 corpus materialization.
//!
//! This is deliberately an artifact generator. It never invokes a model,
//! binder, source mutation path, or hidden judge.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub use crate::e04_authorization::{
    MaterializationAuthorization, MaterializationAuthorizationInput, MaterializerIdentity,
    R1_AUTHORIZATION_ENVELOPE_SCHEMA, R1_AUTHORIZATION_ISSUER, R1_AUTHORIZATION_PURPOSE,
    R1_AUTHORIZATION_SCHEMA, authorize_materialization, materializer_contract_sha256,
    materializer_identity,
};
use crate::e04_authorization::{MaterializationResultBinding, canonical_json_bytes};
use crate::population::{self, EditingPopulationSpec, PopulationSlot};
use crate::{
    BuildSystem, GenerateOptions, RepositoryLayout, TaskFamily, TaskVariant, generate_with_variant,
};

pub const E04_PUBLIC_SCHEMA: &str = "semantic-editing-e04-public-task/0.1";
pub const E04_CONTROLLER_SCHEMA: &str = "semantic-editing-e04-controller/0.2";
pub const E04_MATERIALIZER_IDENTITY_SCHEMA: &str =
    "semantic-editing-e04-r1-materializer-identity/0.1";
pub const E04_MATERIALIZATION_RESULT_SCHEMA: &str =
    "semantic-editing-e04-r1-materialization-result/0.1";
pub const FROZEN_PRODUCT_REVISION: &str = "a6ae1e48359eccef15060c1bb249a648857f30c9";
pub const FROZEN_POPULATION_SHA256: &str =
    "a209f115b0a175bb74859b0539f75932cd664a495332ccf10b634b3cf1c2b9f2";
pub const FROZEN_BINDER_TREE_SHA256: &str =
    "fc349a728c92750e7eb36c39368ef693d708c98badccf4eb9c0a246279474ba4";

#[derive(Debug, Clone)]
pub struct MaterializeOptions {
    pub experiment_root: PathBuf,
    pub population_json: String,
    pub binder_freeze: String,
    pub binder_tree_sha256: String,
    pub population_sha256: String,
    pub gradle_wrapper_assets: Option<GradleWrapperAssets>,
}

#[derive(Debug, Clone)]
pub struct GradleWrapperAssets {
    pub tooling_root: PathBuf,
    pub wrapper_script: PathBuf,
    pub wrapper_jar: PathBuf,
    pub wrapper_properties: PathBuf,
    pub manifest: PathBuf,
    pub codeclew_binary_sha256: String,
    pub typed_goal_catalog_sha256: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ToolingAssetManifest {
    tooling_sha256: BTreeMap<String, String>,
}

#[derive(Debug)]
struct ValidatedGradleWrapperAssets {
    script: PathBuf,
    jar: PathBuf,
    properties: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct PublicTask {
    pub schema: String,
    pub task_id: String,
    pub build_system: BuildSystem,
    pub kotlin_version: String,
    pub task: String,
    pub repository: String,
    pub source_snapshot_sha256: String,
    pub build_command: Vec<String>,
    pub controller_manifest_commitment: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ControllerTask {
    pub schema: String,
    pub task_id: String,
    pub series_id: String,
    pub controller_seed_commitment: String,
    pub slot: PopulationSlot,
    pub seed: u64,
    pub binder_freeze: String,
    pub binder_tree_sha256: String,
    pub population_sha256: String,
    pub required_bindings: Vec<String>,
    pub required_obligations: Vec<String>,
    pub expected_outcome: ExpectedOutcome,
    pub expected_oracle_class: Option<String>,
    pub ambiguous_choices: Vec<Vec<String>>,
    pub refusal_reason: Option<String>,
    pub commitments: Vec<String>,
    pub public_manifest_sha256: String,
    pub commitment: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ExpectedOutcome {
    Bound,
    Ambiguous,
    Refused,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MaterializedExperiment {
    pub agent_root: PathBuf,
    pub controller_root: PathBuf,
    pub tasks: usize,
    pub result: MaterializationResult,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct MaterializerIdentityReport {
    pub schema: String,
    pub materializer: MaterializerIdentity,
    pub materializer_contract_sha256: String,
    pub readiness_graph_sha256: String,
    pub readiness_checker_source_sha256: String,
    pub issuer: String,
    pub purpose: String,
    pub authorization_envelope_schema: String,
    pub authorization_payload_schema: String,
    pub materialization_result_schema: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct AgentPublicMember {
    pub task_id: String,
    pub public_manifest_sha256: String,
    pub repository_source_sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ControllerMember {
    pub task_id: String,
    pub controller_manifest_sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct MaterializationResult {
    pub schema: String,
    pub authorization_envelope_sha256: String,
    pub root_receipt_sha256: String,
    pub decision_freeze_sha256: String,
    pub series_id: String,
    pub output_path: String,
    pub task_count: usize,
    pub agent_public_members: Vec<AgentPublicMember>,
    pub agent_public_set_sha256: String,
    pub controller_members: Vec<ControllerMember>,
    pub controller_set_sha256: String,
    pub r1_public_set_sha256: String,
    pub r1_controller_tree_sha256: String,
}

pub fn materializer_identity_report() -> MaterializerIdentityReport {
    let materializer = materializer_identity();
    MaterializerIdentityReport {
        schema: E04_MATERIALIZER_IDENTITY_SCHEMA.into(),
        materializer_contract_sha256: materializer_contract_sha256(),
        readiness_graph_sha256: materializer.readiness_graph_sha256.clone(),
        readiness_checker_source_sha256: materializer.readiness_checker_source_sha256.clone(),
        materializer,
        issuer: R1_AUTHORIZATION_ISSUER.into(),
        purpose: R1_AUTHORIZATION_PURPOSE.into(),
        authorization_envelope_schema: R1_AUTHORIZATION_ENVELOPE_SCHEMA.into(),
        authorization_payload_schema: R1_AUTHORIZATION_SCHEMA.into(),
        materialization_result_schema: E04_MATERIALIZATION_RESULT_SCHEMA.into(),
    }
}

pub fn canonical_json<T: Serialize>(value: &T) -> Result<String> {
    let bytes = canonical_json_bytes(&serde_json::to_value(value)?);
    String::from_utf8(bytes).context("canonical JSON is not UTF-8")
}

pub fn materialize(
    options: &MaterializeOptions,
    authorization: MaterializationAuthorization,
) -> Result<MaterializedExperiment> {
    authorization.validate_for(options)?;
    validate_materialize_options(options)?;
    materialize_authorized(options, &authorization)
}

fn validate_materialize_options(options: &MaterializeOptions) -> Result<()> {
    if options.binder_freeze != FROZEN_PRODUCT_REVISION
        || options.population_sha256 != FROZEN_POPULATION_SHA256
    {
        bail!("E04 materialization requires the approved binder and population freezes");
    }
    if options.binder_tree_sha256 != FROZEN_BINDER_TREE_SHA256 {
        bail!("E04 requires the canonical frozen binder-tree SHA-256");
    }
    if sha256_hex(options.population_json.as_bytes()) != FROZEN_POPULATION_SHA256 {
        bail!("E04 population JSON differs from the frozen population content");
    }
    Ok(())
}

fn materialize_authorized(
    options: &MaterializeOptions,
    authorization: &MaterializationAuthorization,
) -> Result<MaterializedExperiment> {
    let spec = population::parse_and_validate(&options.population_json)?;
    if spec.slots.len() != 42 {
        bail!("E04 requires exactly 42 frozen slots");
    }
    if options.experiment_root.exists() && options.experiment_root.read_dir()?.next().is_some() {
        bail!("requested experiment root must be absent or empty");
    }
    let gradle_assets = if spec
        .slots
        .iter()
        .any(|slot| slot.build_system == BuildSystem::Gradle)
    {
        Some(validate_gradle_wrapper_assets(options.gradle_wrapper_assets.as_ref().context(
            "Gradle E04 materialization requires explicit named wrapper assets and manifest provenance",
        )?)?)
    } else {
        None
    };
    let agent_root = options.experiment_root.join("agent");
    let controller_root = options.experiment_root.join("controller");
    for slot in &spec.slots {
        materialize_slot(
            &spec,
            slot,
            options,
            authorization,
            gradle_assets.as_ref(),
            &agent_root,
            &controller_root,
        )?;
    }
    let result =
        summarize_materialized_output(&options.experiment_root, &authorization.result_binding())?;
    Ok(MaterializedExperiment {
        agent_root,
        controller_root,
        tasks: spec.slots.len(),
        result,
    })
}

fn materialize_slot(
    spec: &EditingPopulationSpec,
    slot: &PopulationSlot,
    options: &MaterializeOptions,
    authorization: &MaterializationAuthorization,
    gradle_assets: Option<&ValidatedGradleWrapperAssets>,
    agent_root: &Path,
    controller_root: &Path,
) -> Result<()> {
    let base_seed = population::derive_slot_seed(
        spec,
        &options.binder_tree_sha256,
        &options.population_sha256,
        slot,
    )?;
    let slot_key = slot_id(slot);
    let seed = authorization.derive_slot_seed(base_seed, &slot_key);
    let task_id = format!(
        "e04-{}",
        &sha256_hex(format!("{}:{seed}", slot_key).as_bytes())[..16]
    );
    let staging = agent_root.join(".staging").join(&task_id);
    let generated = generate_with_variant(
        &GenerateOptions {
            seed,
            family: TaskFamily::Smoke,
            build_system: slot.build_system,
            output: staging.clone(),
        },
        Some(slot.variant),
    )?;
    if slot.build_system == BuildSystem::Gradle {
        copy_gradle_wrapper(
            gradle_assets.context("validated Gradle wrapper assets are missing")?,
            &generated.agent_dir.join("repository"),
        )?;
    }
    let template = family_template_for(&slot.family, slot.variant, seed)?;
    write_dynamic_template(
        &generated.agent_dir.join("repository"),
        generated.public_manifest.layout,
        &template,
    )?;
    let source_snapshot_sha256 = repository_digest(&generated.agent_dir.join("repository"))?;
    let family = spec
        .families
        .iter()
        .find(|family| family.id == slot.family)
        .context("slot family missing from frozen spec")?;
    let controller_dir = controller_root.join(&task_id);
    fs::create_dir_all(&controller_dir)?;
    let public_path = generated.agent_dir.join("task-manifest.json");
    let mut controller = ControllerTask {
        schema: E04_CONTROLLER_SCHEMA.into(),
        task_id: task_id.clone(),
        series_id: authorization.series_id().into(),
        controller_seed_commitment: authorization.controller_seed_commitment(),
        slot: slot.clone(),
        seed,
        binder_freeze: options.binder_freeze.clone(),
        binder_tree_sha256: options.binder_tree_sha256.clone(),
        population_sha256: options.population_sha256.clone(),
        required_bindings: template.bindings.clone(),
        required_obligations: family.required_obligations.clone(),
        expected_outcome: match slot.variant {
            TaskVariant::Positive => ExpectedOutcome::Bound,
            TaskVariant::Ambiguous => ExpectedOutcome::Ambiguous,
            TaskVariant::MustRefuse => ExpectedOutcome::Refused,
        },
        expected_oracle_class: (slot.variant == TaskVariant::Positive)
            .then(|| "EXTERNAL_SPEC".into()),
        ambiguous_choices: if slot.variant == TaskVariant::Ambiguous {
            template.ambiguities.clone()
        } else {
            Vec::new()
        },
        refusal_reason: (slot.variant == TaskVariant::MustRefuse).then(|| template.refusal.clone()),
        commitments: vec![
            format!("series:{}", authorization.series_id()),
            format!("slot:{slot_key}"),
            format!("seed:{seed}"),
            format!("source: {}", source_snapshot_sha256),
        ],
        public_manifest_sha256: String::new(),
        commitment: String::new(),
    };
    controller.commitment = controller_commitment(&controller)?;
    let public = PublicTask {
        schema: E04_PUBLIC_SCHEMA.into(),
        task_id: task_id.clone(),
        build_system: slot.build_system,
        kotlin_version: generated.public_manifest.kotlin_version,
        task: template.public_task.clone(),
        repository: "repository".into(),
        source_snapshot_sha256,
        build_command: generated.public_manifest.build_command,
        controller_manifest_commitment: controller.commitment.clone(),
    };
    let public_json = serde_json::to_string_pretty(&public)?;
    controller.public_manifest_sha256 = sha256_hex(public_json.as_bytes());
    controller.commitment = controller_commitment(&controller)?;
    let public = PublicTask {
        controller_manifest_commitment: controller.commitment.clone(),
        ..public
    };
    fs::write(public_path, serde_json::to_string_pretty(&public)?)?;
    fs::write(
        controller_dir.join("manifest.json"),
        serde_json::to_string_pretty(&controller)?,
    )?;
    let final_agent = agent_root.join(&task_id);
    fs::create_dir_all(agent_root)?;
    fs::rename(staging.join("agent"), &final_agent)?;
    fs::remove_dir_all(staging)?;
    Ok(())
}

fn slot_id(slot: &PopulationSlot) -> String {
    format!(
        "{}-{}-{}-{}",
        slot.family, slot.variant, slot.build_system, slot.ordinal
    )
}

#[derive(Clone)]
struct DynamicTemplate {
    public_task: String,
    bindings: Vec<String>,
    ambiguities: Vec<Vec<String>>,
    refusal: String,
    kotlin: String,
}

fn family_template_for(family: &str, variant: TaskVariant, seed: u64) -> Result<DynamicTemplate> {
    let token = format!("n{:08x}", (seed as u32) ^ ((seed >> 32) as u32));
    let package = format!("p{token}");
    let name = |prefix: &str| format!("{prefix}{token}");
    let fq = |symbol: &str| format!("{package}.{symbol}");
    let render_bindings = |pairs: &[(&str, String)]| {
        pairs
            .iter()
            .map(|(role, symbol)| format!("{role}={}", fq(symbol)))
            .collect::<Vec<_>>()
    };

    let (public_task, bindings, ambiguities, refusal, body) = match family {
        "producer-transform-consumer" => {
            let context = name("a");
            let transform_a = name("b");
            let transform_b = name("c");
            let workflow = name("d");
            let primary = vec![
                ("CONTEXT_PRODUCER", context.clone()),
                ("TRANSFORMER", transform_a.clone()),
                ("VALUE_EDGE", workflow.clone()),
            ];
            let alternate = vec![
                ("CONTEXT_PRODUCER", context.clone()),
                ("TRANSFORMER", transform_b.clone()),
                ("VALUE_EDGE", workflow.clone()),
            ];
            let (task, alternatives, code) = match variant {
                TaskVariant::Positive => (
                    format!(
                        "In `{}`, arrange for every returned integer to be processed by `{}` with the value from `{}`. The context must be obtained once; preserve order, item count, eagerness, nullability, and observable effects.",
                        fq(&workflow),
                        fq(&transform_a),
                        fq(&context)
                    ),
                    vec![],
                    format!(
                        "fun {context}(): Int = 3\nfun {transform_a}(value: Int, context: Int): Int = value + context\nfun {workflow}(values: List<Int>): List<Int> = values"
                    ),
                ),
                TaskVariant::Ambiguous => (
                    format!(
                        "In `{}`, process every returned integer with one compatible two-argument function and the value from `{}`. Preserve order and item count. The request does not choose between equally valid transformations.",
                        fq(&workflow),
                        fq(&context)
                    ),
                    vec![render_bindings(&primary), render_bindings(&alternate)],
                    format!(
                        "fun {context}(): Int = 3\nfun {transform_a}(value: Int, context: Int): Int = value + context\nfun {transform_b}(value: Int, context: Int): Int = value * context\nfun {workflow}(values: List<Int>): List<Int> = values"
                    ),
                ),
                TaskVariant::MustRefuse => (
                    format!(
                        "In `{}`, process every returned value with `{}` and the context from `{}` while preserving lazy evaluation and effects.",
                        fq(&workflow),
                        fq(&transform_a),
                        fq(&context)
                    ),
                    vec![],
                    format!(
                        "fun {context}(): Int = externalContext{token}()\nexternal fun externalContext{token}(): Int\nfun {transform_a}(value: Int, context: Int): Int = value + context\nfun {workflow}(values: Sequence<Int>): Sequence<Int> = values"
                    ),
                ),
            };
            (
                task,
                render_bindings(&primary),
                alternatives,
                "UNKNOWN_EFFECT_OR_LIFECYCLE",
                code,
            )
        }
        "type-signature-propagation" => {
            let contract = name("e");
            let implementation_a = name("f");
            let implementation_b = name("g");
            let operation = name("h");
            let call_site = name("i");
            let primary = vec![
                ("DECLARATION", format!("{contract}.{operation}")),
                ("OVERRIDE", format!("{implementation_a}.{operation}")),
                ("CALL_SITE", call_site.clone()),
            ];
            let alternate = vec![
                ("DECLARATION", format!("{contract}.{operation}")),
                ("OVERRIDE", format!("{implementation_b}.{operation}")),
                ("CALL_SITE", call_site.clone()),
            ];
            let (task, alternatives, code, refusal) = match variant {
                TaskVariant::Positive => (
                    format!(
                        "Narrow the declared result of `{}.{}` to `String`, and identify the implementation and caller that must stay type-compatible. This internal API is allowed to change.",
                        fq(&contract),
                        operation
                    ),
                    vec![],
                    format!(
                        "internal interface {contract} {{ fun {operation}(): CharSequence }}\ninternal class {implementation_a}: {contract} {{ override fun {operation}(): String = \"{token}\" }}\ninternal fun {call_site}(source: {contract}): CharSequence = source.{operation}()"
                    ),
                    "EXTERNAL_POLICY_ABSENT",
                ),
                TaskVariant::Ambiguous => (
                    format!(
                        "Narrow `{}.{}` to `String` and propagate the change, but the request does not identify which of the two compatible implementations is in scope.",
                        fq(&contract),
                        operation
                    ),
                    vec![render_bindings(&primary), render_bindings(&alternate)],
                    format!(
                        "internal interface {contract} {{ fun {operation}(): CharSequence }}\ninternal class {implementation_a}: {contract} {{ override fun {operation}(): String = \"{token}\" }}\ninternal class {implementation_b}: {contract} {{ override fun {operation}(): String = \"{token}\" }}\ninternal fun {call_site}(source: {contract}): CharSequence = source.{operation}()"
                    ),
                    "EXTERNAL_POLICY_ABSENT",
                ),
                TaskVariant::MustRefuse => (
                    format!(
                        "Narrow the public result of `{}.{}` to `String`; no source or task requirement states whether downstream binary/API compatibility may be broken.",
                        fq(&contract),
                        operation
                    ),
                    vec![],
                    format!(
                        "interface {contract} {{ fun {operation}(): CharSequence }}\nclass {implementation_a}: {contract} {{ override fun {operation}(): String = \"{token}\" }}\nfun {call_site}(source: {contract}): CharSequence = source.{operation}()"
                    ),
                    "EXTERNAL_POLICY_ABSENT",
                ),
            };
            (task, render_bindings(&primary), alternatives, refusal, code)
        }
        "dto-event-api-evolution" => {
            let dto = name("j");
            let field = name("k");
            let factory = name("l");
            let fallback_a = name("m");
            let fallback_b = name("n");
            let primary = vec![
                ("CONTRACT_FIELD", format!("{dto}.{field}")),
                ("CONSTRUCTION_SITE", factory.clone()),
                ("COMPATIBILITY_POLICY", fallback_a.clone()),
            ];
            let alternate = vec![
                ("CONTRACT_FIELD", format!("{dto}.{field}")),
                ("CONSTRUCTION_SITE", factory.clone()),
                ("COMPATIBILITY_POLICY", fallback_b.clone()),
            ];
            let (task, alternatives, code) = match variant {
                TaskVariant::Positive => (
                    format!(
                        "Make `{}.{}` nullable, keep `{}` constructible, and use `{}` as the explicit backward-compatible fallback.",
                        fq(&dto),
                        field,
                        fq(&factory),
                        fq(&fallback_a)
                    ),
                    vec![],
                    format!(
                        "data class {dto}(val {field}: String)\nfun {fallback_a}(): String = \"{token}\"\nfun {factory}(): {dto} = {dto}({fallback_a}())"
                    ),
                ),
                TaskVariant::Ambiguous => (
                    format!(
                        "Make `{}.{}` nullable and keep `{}` constructible. Two documented fallback providers are present, and the request does not select one.",
                        fq(&dto),
                        field,
                        fq(&factory)
                    ),
                    vec![render_bindings(&primary), render_bindings(&alternate)],
                    format!(
                        "data class {dto}(val {field}: String)\nfun {fallback_a}(): String = \"{token}\"\nfun {fallback_b}(): String = \"\"\nfun {factory}(): {dto} = {dto}({fallback_a}())"
                    ),
                ),
                TaskVariant::MustRefuse => (
                    format!(
                        "Make `{}.{}` nullable and preserve compatibility for serialized callers. No default or serialization compatibility policy is specified.",
                        fq(&dto),
                        field
                    ),
                    vec![],
                    format!(
                        "data class {dto}(val {field}: String)\nfun {factory}(value: String): {dto} = {dto}(value)\nfun {fallback_a}(): String = \"{token}\""
                    ),
                ),
            };
            (
                task,
                render_bindings(&primary),
                alternatives,
                "EXTERNAL_POLICY_ABSENT",
                code,
            )
        }
        "persistence-nullability" => {
            let projection_a = name("o");
            let projection_b = name("p");
            let field = name("q");
            let query = name("r");
            let consumer = name("s");
            let primary = vec![
                ("PROJECTION", format!("{projection_a}.{field}")),
                ("DECLARED_TYPE", format!("{projection_a}.{field}")),
                ("QUERY_CONSUMER", consumer.clone()),
            ];
            let alternate = vec![
                ("PROJECTION", format!("{projection_b}.{field}")),
                ("DECLARED_TYPE", format!("{projection_b}.{field}")),
                ("QUERY_CONSUMER", consumer.clone()),
            ];
            let (task, alternatives, code, refusal) = match variant {
                TaskVariant::Positive => (
                    format!(
                        "Align the nullable field `{}.{}` with the row produced by `{}` and the fallback used by `{}`. The repository contract states that a missing database value is allowed.",
                        fq(&projection_a),
                        field,
                        fq(&query),
                        fq(&consumer)
                    ),
                    vec![],
                    format!(
                        "data class {projection_a}(val {field}: String?)\nfun {query}(): {projection_a} = {projection_a}(null)\nfun {consumer}(row: {projection_a}): String = row.{field} ?: \"{token}\""
                    ),
                    "SCHEMA_EVIDENCE_ABSENT",
                ),
                TaskVariant::Ambiguous => (
                    format!(
                        "Align the nullable projection consumed by `{}`. Two same-shaped query results reach it and the request does not identify the intended projection.",
                        fq(&consumer)
                    ),
                    vec![render_bindings(&primary), render_bindings(&alternate)],
                    format!(
                        "data class {projection_a}(val {field}: String?)\ndata class {projection_b}(val {field}: String?)\nfun {query}(first: Boolean): Any = if (first) {projection_a}(null) else {projection_b}(null)\nfun {consumer}(row: Any): String = row.toString()"
                    ),
                    "SCHEMA_EVIDENCE_ABSENT",
                ),
                TaskVariant::MustRefuse => (
                    format!(
                        "Make the projection returned by `{}` non-null and propagate it to `{}`. The SQL is assembled externally and no schema or query-parser evidence is available.",
                        fq(&query),
                        fq(&consumer)
                    ),
                    vec![],
                    format!(
                        "data class {projection_a}(val {field}: String?)\nexternal fun {query}(): {projection_a}\nfun {consumer}(): String = {query}().{field} ?: \"{token}\""
                    ),
                    "QUERY_UNSUPPORTED",
                ),
            };
            (task, render_bindings(&primary), alternatives, refusal, code)
        }
        "configuration-lifecycle" => {
            let config_a = name("t");
            let config_b = name("u");
            let owner = name("v");
            let start = name("w");
            let primary = vec![
                ("CONFIGURATION_PRODUCER", config_a.clone()),
                ("INITIALIZATION_SITE", format!("{owner}.{start}")),
                ("LIFECYCLE_OWNER", owner.clone()),
            ];
            let alternate = vec![
                ("CONFIGURATION_PRODUCER", config_b.clone()),
                ("INITIALIZATION_SITE", format!("{owner}.{start}")),
                ("LIFECYCLE_OWNER", owner.clone()),
            ];
            let (task, alternatives, code) = match variant {
                TaskVariant::Positive => (
                    format!(
                        "Initialize `{}.{}` from `{}` exactly once before the owner is used; preserve initialization order.",
                        fq(&owner),
                        start,
                        fq(&config_a)
                    ),
                    vec![],
                    format!(
                        "fun {config_a}(): String = \"{token}\"\nclass {owner} {{ private lateinit var value: String; fun {start}() {{ value = {config_a}() }}; fun read() = value }}"
                    ),
                ),
                TaskVariant::Ambiguous => (
                    format!(
                        "Initialize `{}.{}` from one compatible configuration source. Two sources are present and the request does not select the lifecycle configuration.",
                        fq(&owner),
                        start
                    ),
                    vec![render_bindings(&primary), render_bindings(&alternate)],
                    format!(
                        "fun {config_a}(): String = \"{token}\"\nfun {config_b}(): String = \"other\"\nclass {owner} {{ private lateinit var value: String; fun {start}() {{ value = \"\" }}; fun read() = value }}"
                    ),
                ),
                TaskVariant::MustRefuse => (
                    format!(
                        "Initialize `{}.{}` from the runtime-selected configuration provider while preserving lifecycle order. The provider is selected through reflection.",
                        fq(&owner),
                        start
                    ),
                    vec![],
                    format!(
                        "fun {config_a}(): String = \"{token}\"\nclass {owner} {{ fun {start}(provider: String): Any = Class.forName(provider).getDeclaredConstructor().newInstance() }}"
                    ),
                ),
            };
            (
                task,
                render_bindings(&primary),
                alternatives,
                "UNRESOLVED_FRAMEWORK_BOUNDARY",
                code,
            )
        }
        "error-retry-resource" => {
            let resource = name("x");
            let run = name("y");
            let retry_a = name("z");
            let retry_b = name("aa");
            let primary = vec![
                ("FAILURE_PATH", run.clone()),
                ("RESOURCE_OWNER", format!("{resource}.close")),
                ("RETRY_OPERATION", retry_a.clone()),
            ];
            let alternate = vec![
                ("FAILURE_PATH", run.clone()),
                ("RESOURCE_OWNER", format!("{resource}.close")),
                ("RETRY_OPERATION", retry_b.clone()),
            ];
            let (task, alternatives, code) = match variant {
                TaskVariant::Positive => (
                    format!(
                        "Route failures in `{}` through `{}` while ensuring `{}` closes once per attempt and preserving retry order and cardinality.",
                        fq(&run),
                        fq(&retry_a),
                        fq(&resource)
                    ),
                    vec![],
                    format!(
                        "class {resource}: AutoCloseable {{ override fun close() {{}}; fun read() = \"{token}\" }}\nfun <T> {retry_a}(block: () -> T): T = block()\nfun {run}(): String = {resource}().use {{ it.read() }}"
                    ),
                ),
                TaskVariant::Ambiguous => (
                    format!(
                        "Route failures in `{}` through one retry operation while preserving resource closure and attempt order. Two compatible retry operations are present and no policy selects one.",
                        fq(&run)
                    ),
                    vec![render_bindings(&primary), render_bindings(&alternate)],
                    format!(
                        "class {resource}: AutoCloseable {{ override fun close() {{}}; fun read() = \"{token}\" }}\nfun <T> {retry_a}(block: () -> T): T = block()\nfun <T> {retry_b}(block: () -> T): T = block()\nfun {run}(): String = {resource}().use {{ it.read() }}"
                    ),
                ),
                TaskVariant::MustRefuse => (
                    format!(
                        "Add retries around `{}` while preserving close timing and attempt order. The number and backoff policy are owned by an external service and are absent from this repository.",
                        fq(&run)
                    ),
                    vec![],
                    format!(
                        "class {resource}: AutoCloseable {{ override fun close() {{}}; fun read() = \"{token}\" }}\nfun <T> {retry_a}(block: () -> T): T = block()\nfun {run}(): String = {resource}().use {{ it.read() }}"
                    ),
                ),
            };
            (
                task,
                render_bindings(&primary),
                alternatives,
                "EXTERNAL_RETRY_POLICY_ABSENT",
                code,
            )
        }
        "test-regression-strengthening" => {
            let behavior = name("ab");
            let oracle_a = name("ac");
            let oracle_b = name("ad");
            let primary = vec![
                ("BEHAVIOR_UNDER_TEST", behavior.clone()),
                ("INDEPENDENT_ORACLE", oracle_a.clone()),
                ("PRODUCTION_CONTRACT", behavior.clone()),
            ];
            let alternate = vec![
                ("BEHAVIOR_UNDER_TEST", behavior.clone()),
                ("INDEPENDENT_ORACLE", oracle_b.clone()),
                ("PRODUCTION_CONTRACT", behavior.clone()),
            ];
            let (task, alternatives, code, refusal) = match variant {
                TaskVariant::Positive => (
                    format!(
                        "Strengthen the regression around `{}` so omission of its behavior is detected using the independent expected value in `{}`; preserve the production contract.",
                        fq(&behavior),
                        fq(&oracle_a)
                    ),
                    vec![],
                    format!(
                        "fun {behavior}(value: String): String = value.uppercase()\nconst val {oracle_a}: String = \"{token}\"\nfun weak{token}(): Boolean = {behavior}(\"{token}\") .isNotEmpty()"
                    ),
                    "BUSINESS_ORACLE_ABSENT",
                ),
                TaskVariant::Ambiguous => (
                    format!(
                        "Strengthen the regression around `{}` using an independent expected value. Two incompatible expected values are documented and the request does not select one.",
                        fq(&behavior)
                    ),
                    vec![render_bindings(&primary), render_bindings(&alternate)],
                    format!(
                        "fun {behavior}(value: String): String = value.uppercase()\nconst val {oracle_a}: String = \"{token}\"\nconst val {oracle_b}: String = \"OTHER\"\nfun weak{token}(): Boolean = {behavior}(\"{token}\").isNotEmpty()"
                    ),
                    "BUSINESS_ORACLE_ABSENT",
                ),
                TaskVariant::MustRefuse => (
                    format!(
                        "Strengthen the regression around `{}` with an independent behavioral oracle. The only expected-value helper calls the production function itself.",
                        fq(&behavior)
                    ),
                    vec![],
                    format!(
                        "fun {behavior}(value: String): String = value.uppercase()\nfun {oracle_a}(value: String): String = {behavior}(value)\nfun weak{token}(): Boolean = {oracle_a}(\"{token}\") == {behavior}(\"{token}\")"
                    ),
                    "SELF_CONFIRMING_ORACLE",
                ),
            };
            (task, render_bindings(&primary), alternatives, refusal, code)
        }
        _ => bail!("unknown frozen E04 family"),
    };

    Ok(DynamicTemplate {
        public_task,
        bindings,
        ambiguities,
        refusal: refusal.into(),
        kotlin: format!("package {package}\n\n{body}\n"),
    })
}

fn write_dynamic_template(
    repository: &Path,
    layout: RepositoryLayout,
    template: &DynamicTemplate,
) -> Result<()> {
    let package = template
        .kotlin
        .lines()
        .next()
        .context("template package missing")?
        .trim_start_matches("package ");
    let source_root = match layout {
        RepositoryLayout::Flat => repository.to_path_buf(),
        RepositoryLayout::Module => {
            let mut modules = fs::read_dir(repository)?
                .filter_map(|entry| entry.ok())
                .map(|entry| entry.path())
                .filter(|path| {
                    path.is_dir()
                        && (path.join("build.gradle.kts").is_file()
                            || path.join("pom.xml").is_file())
                })
                .collect::<Vec<_>>();
            if modules.len() != 1 {
                bail!("generated module layout must contain exactly one build module");
            }
            modules.remove(0)
        }
    };
    let file = source_root
        .join("src/main/kotlin")
        .join(package)
        .join("Workflow.kt");
    if let Some(parent) = file.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(file, &template.kotlin)?;
    Ok(())
}

fn controller_commitment(value: &ControllerTask) -> Result<String> {
    let mut stable = value.clone();
    stable.public_manifest_sha256.clear();
    stable.commitment.clear();
    Ok(sha256_hex(serde_json::to_vec(&stable)?.as_slice()))
}
fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn contained_regular_file(root: &Path, path: &Path, label: &str) -> Result<PathBuf> {
    let metadata = fs::symlink_metadata(path)
        .with_context(|| format!("missing named Gradle wrapper {label} {}", path.display()))?;
    if metadata.file_type().is_symlink() || !metadata.file_type().is_file() {
        bail!(
            "Gradle wrapper {label} must be a regular non-symlink file: {}",
            path.display()
        );
    }
    let canonical = fs::canonicalize(path)?;
    if !canonical.starts_with(root) {
        bail!(
            "Gradle wrapper {label} escapes immutable tooling root: {}",
            path.display()
        );
    }
    Ok(canonical)
}

fn sha256_file(path: &Path) -> Result<String> {
    Ok(sha256_hex(&fs::read(path)?))
}

fn jar_contains_wrapper_main(bytes: &[u8]) -> bool {
    let Some(eocd) = bytes.windows(4).rposition(|window| window == b"PK\x05\x06") else {
        return false;
    };
    if eocd + 20 > bytes.len() {
        return false;
    }
    let central_size = u32::from_le_bytes(bytes[eocd + 12..eocd + 16].try_into().unwrap()) as usize;
    let central_offset =
        u32::from_le_bytes(bytes[eocd + 16..eocd + 20].try_into().unwrap()) as usize;
    let Some(end) = central_offset.checked_add(central_size) else {
        return false;
    };
    if end > bytes.len() {
        return false;
    }
    let mut cursor = central_offset;
    while cursor < end {
        if cursor + 46 > end || &bytes[cursor..cursor + 4] != b"PK\x01\x02" {
            return false;
        }
        let name_len =
            u16::from_le_bytes(bytes[cursor + 28..cursor + 30].try_into().unwrap()) as usize;
        let extra_len =
            u16::from_le_bytes(bytes[cursor + 30..cursor + 32].try_into().unwrap()) as usize;
        let comment_len =
            u16::from_le_bytes(bytes[cursor + 32..cursor + 34].try_into().unwrap()) as usize;
        let name_start = cursor + 46;
        let Some(next) = name_start
            .checked_add(name_len)
            .and_then(|value| value.checked_add(extra_len))
            .and_then(|value| value.checked_add(comment_len))
        else {
            return false;
        };
        if next > end {
            return false;
        }
        if &bytes[name_start..name_start + name_len]
            == b"org/gradle/wrapper/GradleWrapperMain.class"
        {
            return true;
        }
        cursor = next;
    }
    false
}

fn validate_gradle_wrapper_assets(
    assets: &GradleWrapperAssets,
) -> Result<ValidatedGradleWrapperAssets> {
    if [
        &assets.codeclew_binary_sha256,
        &assets.typed_goal_catalog_sha256,
    ]
    .iter()
    .any(|value| value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()))
    {
        bail!("Codeclew binary/catalog provenance must use SHA-256 hex digests");
    }
    let root_metadata = fs::symlink_metadata(&assets.tooling_root)?;
    if root_metadata.file_type().is_symlink() || !root_metadata.is_dir() {
        bail!("Gradle immutable tooling root must be a regular directory");
    }
    let root = fs::canonicalize(&assets.tooling_root)?;
    let script = contained_regular_file(&root, &assets.wrapper_script, "script")?;
    let jar = contained_regular_file(&root, &assets.wrapper_jar, "JAR")?;
    let properties = contained_regular_file(&root, &assets.wrapper_properties, "properties")?;
    let manifest_path = contained_regular_file(&root, &assets.manifest, "manifest")?;
    let manifest: ToolingAssetManifest = serde_json::from_slice(&fs::read(manifest_path)?)?;
    let named = [
        ("gradlew", &script),
        ("gradle/wrapper/gradle-wrapper.jar", &jar),
        ("gradle/wrapper/gradle-wrapper.properties", &properties),
    ];
    for (name, path) in named {
        let expected = manifest
            .tooling_sha256
            .get(name)
            .with_context(|| format!("tooling manifest does not bind named asset {name}"))?;
        if expected.len() != 64 || !expected.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            bail!("tooling manifest has invalid SHA-256 for {name}")
        }
        let actual = sha256_file(path)?;
        if &actual != expected {
            bail!("tooling manifest digest mismatch for {name}")
        }
    }
    let jar_bytes = fs::read(&jar)?;
    if !jar_bytes.starts_with(b"PK\x03\x04") || !jar_contains_wrapper_main(&jar_bytes) {
        bail!("Gradle wrapper JAR is not a valid wrapper ZIP containing GradleWrapperMain.class");
    }
    let jar_sha = sha256_file(&jar)?;
    if jar_sha == assets.codeclew_binary_sha256 || jar_sha == assets.typed_goal_catalog_sha256 {
        bail!("Gradle wrapper JAR digest collides with Codeclew binary/catalog provenance");
    }
    let script_bytes = fs::read(&script)?;
    if script_bytes.contains(&0)
        || !(script_bytes.starts_with(b"#!/bin/sh\n")
            || script_bytes.starts_with(b"#!/usr/bin/env sh\n"))
        || std::str::from_utf8(&script_bytes).is_err()
    {
        bail!("Gradle wrapper script must be a recognized non-binary sh script");
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if fs::metadata(&script)?.permissions().mode() & 0o111 == 0 {
            bail!("Gradle wrapper script must have an executable bit");
        }
    }
    let properties_text = fs::read_to_string(&properties)?;
    let expected_distribution =
        "distributionUrl=https\\://services.gradle.org/distributions/gradle-9.6.1-bin.zip";
    if !properties_text
        .lines()
        .any(|line| line == expected_distribution)
    {
        bail!("Gradle wrapper distribution URL/version is not exactly Gradle 9.6.1 bin");
    }
    Ok(ValidatedGradleWrapperAssets {
        script,
        jar,
        properties,
    })
}

fn copy_gradle_wrapper(assets: &ValidatedGradleWrapperAssets, repository: &Path) -> Result<()> {
    for (relative, source) in [
        ("gradlew", &assets.script),
        ("gradle/wrapper/gradle-wrapper.jar", &assets.jar),
        (
            "gradle/wrapper/gradle-wrapper.properties",
            &assets.properties,
        ),
    ] {
        let destination = repository.join(relative);
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::copy(source, &destination)?;
    }
    Ok(())
}

fn repository_digest(root: &Path) -> Result<String> {
    let mut files = Vec::new();
    collect_files(root, root, &mut files)?;
    files.sort();
    let mut digest = Sha256::new();
    for path in files {
        digest.update(path.as_bytes());
        digest.update([0]);
        digest.update(fs::read(root.join(path))?);
        digest.update([0]);
    }
    Ok(hex::encode(digest.finalize()))
}

fn summarize_materialized_output(
    experiment_root: &Path,
    binding: &MaterializationResultBinding,
) -> Result<MaterializationResult> {
    let members = inspect_materialized_member_sets(experiment_root, &binding.series_id)?;
    if members.canonical_root != binding.output_path {
        bail!("materialized experiment root differs from authorized output path");
    }
    Ok(MaterializationResult {
        schema: E04_MATERIALIZATION_RESULT_SCHEMA.into(),
        authorization_envelope_sha256: binding.authorization_envelope_sha256.clone(),
        root_receipt_sha256: binding.root_receipt_sha256.clone(),
        decision_freeze_sha256: binding.decision_freeze_sha256.clone(),
        series_id: binding.series_id.clone(),
        output_path: members.canonical_root.to_string_lossy().into_owned(),
        task_count: 42,
        agent_public_members: members.agent_public_members,
        agent_public_set_sha256: members.agent_public_set_sha256,
        controller_members: members.controller_members,
        controller_set_sha256: members.controller_set_sha256,
        r1_public_set_sha256: members.r1_public_set_sha256,
        r1_controller_tree_sha256: members.r1_controller_tree_sha256,
    })
}

pub(crate) struct E04MemberSets {
    pub canonical_root: PathBuf,
    pub agent_public_members: Vec<AgentPublicMember>,
    pub agent_public_set_sha256: String,
    pub controller_members: Vec<ControllerMember>,
    pub controller_set_sha256: String,
    pub r1_public_set_sha256: String,
    pub r1_controller_tree_sha256: String,
}

pub(crate) fn inspect_materialized_member_sets(
    experiment_root: &Path,
    expected_series_id: &str,
) -> Result<E04MemberSets> {
    let root_metadata =
        fs::symlink_metadata(experiment_root).context("materialized experiment root is missing")?;
    if root_metadata.file_type().is_symlink() || !root_metadata.is_dir() {
        bail!("materialized experiment root must be a real directory");
    }
    let canonical_root = fs::canonicalize(experiment_root)?;
    let agent_dirs = exact_task_directories(&canonical_root.join("agent"), "agent")?;
    let controller_dirs = exact_task_directories(&canonical_root.join("controller"), "controller")?;
    if agent_dirs.len() != 42 || controller_dirs.len() != 42 {
        bail!("materialization result requires exactly 42 agent and 42 controller tasks");
    }
    if agent_dirs.keys().collect::<Vec<_>>() != controller_dirs.keys().collect::<Vec<_>>() {
        bail!("materialization agent/controller task ID sets differ");
    }

    let mut agent_public_members = Vec::with_capacity(42);
    let mut controller_members = Vec::with_capacity(42);
    for (task_id, agent_dir) in agent_dirs {
        let controller_dir = controller_dirs
            .get(&task_id)
            .context("controller task disappeared during result validation")?;
        let public_path =
            exact_regular_file(&agent_dir.join("task-manifest.json"), "public manifest")?;
        let controller_path =
            exact_regular_file(&controller_dir.join("manifest.json"), "controller manifest")?;
        let public_bytes = fs::read(&public_path)?;
        let controller_bytes = fs::read(&controller_path)?;
        let public: PublicTask =
            serde_json::from_slice(&public_bytes).context("invalid public E04 manifest")?;
        let controller: ControllerTask =
            serde_json::from_slice(&controller_bytes).context("invalid controller E04 manifest")?;
        if public.schema != E04_PUBLIC_SCHEMA
            || controller.schema != E04_CONTROLLER_SCHEMA
            || public.task_id != task_id
            || controller.task_id != task_id
            || controller.series_id != expected_series_id
            || public.repository != "repository"
            || public.controller_manifest_commitment != controller.commitment
            || controller_commitment(&controller)? != controller.commitment
            || controller.public_manifest_sha256 != sha256_hex(&public_bytes)
        {
            bail!("materialization manifest authority binding mismatch for {task_id}");
        }
        let repository = agent_dir.join("repository");
        let repository_metadata = fs::symlink_metadata(&repository)
            .context("materialized agent repository is missing")?;
        if repository_metadata.file_type().is_symlink() || !repository_metadata.is_dir() {
            bail!("materialized agent repository must be a real directory");
        }
        let source_sha = repository_digest(&repository)?;
        if source_sha != public.source_snapshot_sha256 {
            bail!("materialized repository source digest mismatch for {task_id}");
        }
        agent_public_members.push(AgentPublicMember {
            task_id: task_id.clone(),
            public_manifest_sha256: sha256_hex(&public_bytes),
            repository_source_sha256: source_sha,
        });
        controller_members.push(ControllerMember {
            task_id,
            controller_manifest_sha256: sha256_hex(&controller_bytes),
        });
    }
    let agent_public_set_sha256 = sha256_hex(&canonical_json_bytes(&serde_json::to_value(
        &agent_public_members,
    )?));
    let controller_set_sha256 = sha256_hex(&canonical_json_bytes(&serde_json::to_value(
        &controller_members,
    )?));
    let public_envelope_members = agent_public_members
        .iter()
        .map(|member| {
            serde_json::json!({
                "taskId":member.task_id,
                "manifestSha256":member.public_manifest_sha256,
                "sourceSha256":member.repository_source_sha256,
            })
        })
        .collect::<Vec<_>>();
    let r1_public_set_sha256 = sha256_hex(&canonical_json_bytes(&serde_json::json!({
        "schema":"e04-public-set-envelope/0.1","series":"R1",
        "root":canonical_root.to_string_lossy(),"members":public_envelope_members
    })));
    let controller_envelope_members = controller_members
        .iter()
        .map(|member| {
            serde_json::json!({
                "taskId":member.task_id,
                "manifestSha256":member.controller_manifest_sha256,
            })
        })
        .collect::<Vec<_>>();
    let r1_controller_tree_sha256 = sha256_hex(&canonical_json_bytes(&serde_json::json!({
        "schema":"e04-controller-set-envelope/0.1","series":"R1",
        "root":canonical_root.to_string_lossy(),"members":controller_envelope_members
    })));
    Ok(E04MemberSets {
        canonical_root,
        agent_public_members,
        agent_public_set_sha256,
        controller_members,
        controller_set_sha256,
        r1_public_set_sha256,
        r1_controller_tree_sha256,
    })
}

fn exact_task_directories(root: &Path, label: &str) -> Result<BTreeMap<String, PathBuf>> {
    let metadata =
        fs::symlink_metadata(root).with_context(|| format!("materialized {label} root missing"))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        bail!("materialized {label} root must be a real directory");
    }
    let mut result = BTreeMap::new();
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let path = entry.path();
        let file_type = entry.file_type()?;
        if file_type.is_symlink() || !file_type.is_dir() {
            bail!("materialized {label} root contains a non-task entry");
        }
        let task_id = entry
            .file_name()
            .into_string()
            .map_err(|_| anyhow::anyhow!("materialized task ID is not UTF-8"))?;
        if task_id.is_empty() || result.insert(task_id, path).is_some() {
            bail!("materialized {label} task IDs are invalid or duplicated");
        }
    }
    Ok(result)
}

fn exact_regular_file(path: &Path, label: &str) -> Result<PathBuf> {
    let metadata =
        fs::symlink_metadata(path).with_context(|| format!("materialized {label} missing"))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        bail!("materialized {label} must be a regular non-symlink file");
    }
    fs::canonicalize(path).with_context(|| format!("cannot canonicalize materialized {label}"))
}

fn collect_files(root: &Path, directory: &Path, files: &mut Vec<String>) -> Result<()> {
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let path = entry.path();
        let file_type = entry.file_type()?;
        if file_type.is_symlink() {
            bail!("E04 repository snapshots do not permit symlinks");
        } else if file_type.is_dir() {
            collect_files(root, &path, files)?;
        } else if file_type.is_file() {
            files.push(
                path.strip_prefix(root)
                    .context("repository file escapes root")?
                    .to_string_lossy()
                    .into_owned(),
            );
        } else {
            bail!("E04 repository snapshots require regular files");
        }
    }
    Ok(())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::process::Command;
    const SPEC: &str =
        include_str!("../../../benchmarks/semantic-change/editing-population-v1.json");

    pub(crate) fn materialization_result_fixture()
    -> (tempfile::TempDir, MaterializationResultBinding) {
        let temporary = tempfile::tempdir().unwrap();
        let output = temporary.path().join("result");
        let agent = output.join("agent");
        let controller_root = output.join("controller");
        fs::create_dir_all(&agent).unwrap();
        fs::create_dir_all(&controller_root).unwrap();
        let population = population::parse_and_validate(SPEC).unwrap();
        let obligations = population
            .families
            .iter()
            .map(|family| (family.id.clone(), family.required_obligations.clone()))
            .collect::<BTreeMap<_, _>>();
        let slots = population.slots;
        let series_id = "a".repeat(64);
        for (index, slot) in slots.into_iter().enumerate() {
            let slot_variant = slot.variant;
            let required_obligations = obligations.get(&slot.family).unwrap().clone();
            let task_id = format!("e04-result-{index:02}");
            let agent_dir = agent.join(&task_id);
            let repository = agent_dir.join("repository");
            let controller_dir = controller_root.join(&task_id);
            fs::create_dir_all(&repository).unwrap();
            fs::create_dir_all(&controller_dir).unwrap();
            fs::write(
                repository.join("source.kt"),
                format!("package p\nfun a(): Int = {index}\nfun b(): Int = {index}\n"),
            )
            .unwrap();
            let source_snapshot_sha256 = repository_digest(&repository).unwrap();
            let mut controller = ControllerTask {
                schema: E04_CONTROLLER_SCHEMA.into(),
                task_id: task_id.clone(),
                series_id: series_id.clone(),
                controller_seed_commitment: "b".repeat(64),
                slot,
                seed: index as u64,
                binder_freeze: FROZEN_PRODUCT_REVISION.into(),
                binder_tree_sha256: FROZEN_BINDER_TREE_SHA256.into(),
                population_sha256: FROZEN_POPULATION_SHA256.into(),
                required_bindings: vec!["DECLARATION=p.a".into()],
                required_obligations,
                expected_outcome: match slot_variant {
                    TaskVariant::Positive => ExpectedOutcome::Bound,
                    TaskVariant::Ambiguous => ExpectedOutcome::Ambiguous,
                    TaskVariant::MustRefuse => ExpectedOutcome::Refused,
                },
                expected_oracle_class: (slot_variant == TaskVariant::Positive)
                    .then(|| "EXTERNAL_SPEC".into()),
                ambiguous_choices: if slot_variant == TaskVariant::Ambiguous {
                    vec![
                        vec!["DECLARATION=p.a".into()],
                        vec!["DECLARATION=p.b".into()],
                    ]
                } else {
                    Vec::new()
                },
                refusal_reason: (slot_variant == TaskVariant::MustRefuse)
                    .then(|| "INCOMPLETE_SEMANTIC_EVIDENCE".into()),
                commitments: vec![],
                public_manifest_sha256: String::new(),
                commitment: String::new(),
            };
            controller.commitment = controller_commitment(&controller).unwrap();
            let public = PublicTask {
                schema: E04_PUBLIC_SCHEMA.into(),
                task_id: task_id.clone(),
                build_system: controller.slot.build_system,
                kotlin_version: "2.4.10".into(),
                task: "Neutral result fixture.".into(),
                repository: "repository".into(),
                source_snapshot_sha256,
                build_command: vec![],
                controller_manifest_commitment: controller.commitment.clone(),
            };
            let public_bytes = serde_json::to_string_pretty(&public).unwrap().into_bytes();
            controller.public_manifest_sha256 = sha256_hex(&public_bytes);
            fs::write(agent_dir.join("task-manifest.json"), public_bytes).unwrap();
            fs::write(
                controller_dir.join("manifest.json"),
                serde_json::to_string_pretty(&controller).unwrap(),
            )
            .unwrap();
        }
        let output = fs::canonicalize(output).unwrap();
        (
            temporary,
            MaterializationResultBinding {
                authorization_envelope_sha256: "c".repeat(64),
                root_receipt_sha256: "d".repeat(64),
                decision_freeze_sha256: "e".repeat(64),
                series_id,
                output_path: output,
            },
        )
    }

    #[test]
    fn r1_materialization_result_is_canonical_and_recomputable() {
        let (_temporary, binding) = materialization_result_fixture();
        let result = summarize_materialized_output(&binding.output_path, &binding).unwrap();
        assert_eq!(result.task_count, 42);
        assert_eq!(result.agent_public_members.len(), 42);
        assert_eq!(result.controller_members.len(), 42);
        assert!(
            result
                .agent_public_members
                .windows(2)
                .all(|pair| pair[0].task_id < pair[1].task_id)
        );
        let canonical = canonical_json(&result).unwrap();
        assert_eq!(
            serde_json::from_str::<MaterializationResult>(&canonical).unwrap(),
            result
        );
    }

    #[test]
    fn r1_materialization_result_refuses_partial_and_malformed_packages() {
        let (_partial_temporary, partial) = materialization_result_fixture();
        fs::remove_dir_all(partial.output_path.join("controller/e04-result-41")).unwrap();
        assert!(summarize_materialized_output(&partial.output_path, &partial).is_err());

        let (_malformed_temporary, malformed) = materialization_result_fixture();
        fs::write(
            malformed
                .output_path
                .join("agent/e04-result-00/task-manifest.json"),
            b"{}\n",
        )
        .unwrap();
        assert!(summarize_materialized_output(&malformed.output_path, &malformed).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn r1_materialization_result_refuses_symlinked_output_evidence() {
        use std::os::unix::fs::symlink;
        let (temporary, binding) = materialization_result_fixture();
        let manifest = binding
            .output_path
            .join("controller/e04-result-00/manifest.json");
        let external = temporary.path().join("external-controller.json");
        fs::rename(&manifest, &external).unwrap();
        symlink(&external, &manifest).unwrap();
        assert!(summarize_materialized_output(&binding.output_path, &binding).is_err());
    }

    fn repository_wrapper_assets() -> GradleWrapperAssets {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        GradleWrapperAssets {
            tooling_root: root.clone(),
            wrapper_script: root.join("gradlew"),
            wrapper_jar: root.join("gradle/wrapper/gradle-wrapper.jar"),
            wrapper_properties: root.join("gradle/wrapper/gradle-wrapper.properties"),
            manifest: root.join("benchmarks/semantic-change/e04-freeze.json"),
            codeclew_binary_sha256: "0".repeat(64),
            typed_goal_catalog_sha256: "1".repeat(64),
        }
    }

    #[test]
    fn post_freeze_plan_has_all_slots_but_tests_do_not_materialize_them() {
        let spec = population::parse_and_validate(SPEC).unwrap();
        assert_eq!(spec.slots.len(), 42);
    }

    #[test]
    fn wrong_freeze_is_rejected_before_any_materialization() {
        let root = tempfile::tempdir().unwrap();
        let error = validate_materialize_options(&MaterializeOptions {
            experiment_root: root.path().join("out"),
            population_json: SPEC.into(),
            binder_freeze: "0".repeat(40),
            binder_tree_sha256: "0".repeat(64),
            population_sha256: FROZEN_POPULATION_SHA256.into(),
            gradle_wrapper_assets: None,
        })
        .unwrap_err();
        assert!(error.to_string().contains("approved binder"));
    }

    #[test]
    fn r1_materialization_rejects_population_bytes_not_bound_by_the_freeze() {
        let root = tempfile::tempdir().unwrap();
        let error = validate_materialize_options(&MaterializeOptions {
            experiment_root: root.path().join("out"),
            population_json: format!("{SPEC}\n"),
            binder_freeze: FROZEN_PRODUCT_REVISION.into(),
            binder_tree_sha256: FROZEN_BINDER_TREE_SHA256.into(),
            population_sha256: FROZEN_POPULATION_SHA256.into(),
            gradle_wrapper_assets: None,
        })
        .unwrap_err();
        assert!(error.to_string().contains("population JSON differs"));
    }

    #[test]
    fn gradle_wrapper_assets_reject_old_binary_as_jar_and_accept_named_manifest_assets() {
        let valid = repository_wrapper_assets();
        let validated = validate_gradle_wrapper_assets(&valid).unwrap();
        assert_eq!(
            sha256_file(&validated.jar).unwrap(),
            "497c8c2a7e5031f6aa847f88104aa80a93532ec32ee17bdb8d1d2f67a194a9c7"
        );

        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path();
        fs::create_dir_all(root.join("gradle/wrapper")).unwrap();
        fs::copy(&valid.wrapper_script, root.join("gradlew")).unwrap();
        fs::copy(
            &valid.wrapper_properties,
            root.join("gradle/wrapper/gradle-wrapper.properties"),
        )
        .unwrap();
        let old_binary = b"\xcf\xfa\xed\xfeold-clew-mach-o-not-a-jar";
        fs::write(root.join("gradle/wrapper/gradle-wrapper.jar"), old_binary).unwrap();
        let manifest = serde_json::json!({"toolingSha256":{
            "gradlew":sha256_file(&root.join("gradlew")).unwrap(),
            "gradle/wrapper/gradle-wrapper.jar":sha256_hex(old_binary),
            "gradle/wrapper/gradle-wrapper.properties":sha256_file(&root.join("gradle/wrapper/gradle-wrapper.properties")).unwrap()
        }});
        fs::write(
            root.join("manifest.json"),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();
        let invalid = GradleWrapperAssets {
            tooling_root: root.into(),
            wrapper_script: root.join("gradlew"),
            wrapper_jar: root.join("gradle/wrapper/gradle-wrapper.jar"),
            wrapper_properties: root.join("gradle/wrapper/gradle-wrapper.properties"),
            manifest: root.join("manifest.json"),
            codeclew_binary_sha256: sha256_hex(old_binary),
            typed_goal_catalog_sha256: "2".repeat(64),
        };
        assert!(
            validate_gradle_wrapper_assets(&invalid)
                .unwrap_err()
                .to_string()
                .contains("not a valid wrapper ZIP")
        );
    }

    #[test]
    fn public_manifest_shape_has_no_hidden_slot_or_oracle_fields() {
        let public = PublicTask {
            schema: E04_PUBLIC_SCHEMA.into(),
            task_id: "e04-opaque".into(),
            build_system: BuildSystem::Maven,
            kotlin_version: "2.1.0".into(),
            task: "Review the requested workflow.".into(),
            repository: "repository".into(),
            source_snapshot_sha256: "a".repeat(64),
            build_command: vec!["mvn".into(), "test".into()],
            controller_manifest_commitment: "b".repeat(64),
        };
        let json = serde_json::to_string(&public).unwrap();
        for forbidden in [
            "family",
            "variant",
            "obligation",
            "oracle",
            "refusal",
            "seed",
        ] {
            assert!(!json.to_ascii_lowercase().contains(forbidden));
        }
    }

    #[test]
    fn dynamic_templates_have_variant_evidence_and_seeded_symbols() {
        let positive =
            family_template_for("producer-transform-consumer", TaskVariant::Positive, 11).unwrap();
        let ambiguous =
            family_template_for("producer-transform-consumer", TaskVariant::Ambiguous, 11).unwrap();
        let refused =
            family_template_for("producer-transform-consumer", TaskVariant::MustRefuse, 11)
                .unwrap();
        assert_ne!(positive.kotlin, ambiguous.kotlin);
        assert_ne!(ambiguous.kotlin, refused.kotlin);
        assert!(ambiguous.ambiguities.iter().flatten().all(|binding| {
            binding.split('=').nth(1).is_some_and(|symbol| {
                ambiguous
                    .kotlin
                    .contains(symbol.rsplit('.').next().unwrap())
            })
        }));
        assert!(refused.kotlin.contains("external"));
        let changed_seed =
            family_template_for("producer-transform-consumer", TaskVariant::Positive, 12).unwrap();
        assert_ne!(positive.kotlin, changed_seed.kotlin);
        for binding in &positive.bindings {
            assert!(
                positive.kotlin.contains(
                    binding
                        .rsplit('=')
                        .next()
                        .unwrap()
                        .rsplit('.')
                        .next()
                        .unwrap()
                )
            );
        }
        for forbidden in [
            "CONTEXT_PRODUCER",
            "TRANSFORMER",
            "VALUE_EDGE",
            "MULTIPLE_COMPATIBLE",
            "SERIALIZATION_POLICY",
            "QUERY_PARSER",
        ] {
            assert!(!positive.kotlin.contains(forbidden));
            assert!(!positive.public_task.contains(forbidden));
        }
        let shapes = [
            "producer-transform-consumer",
            "type-signature-propagation",
            "dto-event-api-evolution",
            "persistence-nullability",
            "configuration-lifecycle",
            "error-retry-resource",
            "test-regression-strengthening",
        ]
        .into_iter()
        .map(|family| {
            family_template_for(family, TaskVariant::Positive, 11)
                .unwrap()
                .kotlin
        })
        .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(shapes.len(), 7);

        let refusal_codes = [
            "UNKNOWN_EFFECT_OR_LIFECYCLE",
            "EXTERNAL_POLICY_ABSENT",
            "SCHEMA_EVIDENCE_ABSENT",
            "QUERY_UNSUPPORTED",
            "UNRESOLVED_FRAMEWORK_BOUNDARY",
            "EXTERNAL_RETRY_POLICY_ABSENT",
            "SELF_CONFIRMING_ORACLE",
        ]
        .into_iter()
        .collect::<std::collections::BTreeSet<_>>();
        for family in [
            "producer-transform-consumer",
            "type-signature-propagation",
            "dto-event-api-evolution",
            "persistence-nullability",
            "configuration-lifecycle",
            "error-retry-resource",
            "test-regression-strengthening",
        ] {
            let template = family_template_for(family, TaskVariant::MustRefuse, 11).unwrap();
            assert!(refusal_codes.contains(template.refusal.as_str()));
            assert!(!template.public_task.contains(&template.refusal));
            assert!(!template.kotlin.contains(&template.refusal));
        }
    }

    #[test]
    fn sample_module_layout_places_dynamic_source_inside_the_build_module() {
        let seed = (0..100)
            .find(|seed| {
                super::super::repository_layout(*seed, TaskFamily::Smoke, BuildSystem::Maven)
                    == RepositoryLayout::Module
            })
            .unwrap();
        let temporary = tempfile::tempdir().unwrap();
        let generated = generate_with_variant(
            &GenerateOptions {
                seed,
                family: TaskFamily::Smoke,
                build_system: BuildSystem::Maven,
                output: temporary.path().join("sample"),
            },
            Some(TaskVariant::Positive),
        )
        .unwrap();
        let template =
            family_template_for("type-signature-propagation", TaskVariant::Positive, seed).unwrap();
        let repository = generated.agent_dir.join("repository");
        write_dynamic_template(&repository, RepositoryLayout::Module, &template).unwrap();
        assert!(!repository.join("src/main/kotlin").exists());
        assert_eq!(
            fs::read_dir(&repository)
                .unwrap()
                .filter_map(|entry| entry.ok())
                .filter(|entry| entry.path().join("src/main/kotlin").is_dir())
                .count(),
            1
        );
    }

    #[test]
    fn public_and_controller_commitments_finalize_without_a_hash_cycle() {
        let slot = population::parse_and_validate(SPEC).unwrap().slots[0].clone();
        let mut controller = ControllerTask {
            schema: E04_CONTROLLER_SCHEMA.into(),
            task_id: "e04-0000000000000000".into(),
            series_id: "c".repeat(64),
            controller_seed_commitment: "d".repeat(64),
            slot,
            seed: 0,
            binder_freeze: FROZEN_PRODUCT_REVISION.into(),
            binder_tree_sha256: FROZEN_BINDER_TREE_SHA256.into(),
            population_sha256: FROZEN_POPULATION_SHA256.into(),
            required_bindings: vec!["VALUE_EDGE=p.a".into()],
            required_obligations: vec!["prove transform placement".into()],
            expected_outcome: ExpectedOutcome::Bound,
            expected_oracle_class: Some("EXTERNAL_SPEC".into()),
            ambiguous_choices: vec![],
            refusal_reason: None,
            commitments: vec![],
            public_manifest_sha256: String::new(),
            commitment: String::new(),
        };
        let first = controller_commitment(&controller).unwrap();
        controller.commitment = first.clone();
        let public = PublicTask {
            schema: E04_PUBLIC_SCHEMA.into(),
            task_id: controller.task_id.clone(),
            build_system: BuildSystem::Gradle,
            kotlin_version: "2.1.21".into(),
            task: "Change p.a.".into(),
            repository: "repository".into(),
            source_snapshot_sha256: "a".repeat(64),
            build_command: vec![],
            controller_manifest_commitment: first.clone(),
        };
        let public_json = serde_json::to_string_pretty(&public).unwrap();
        controller.public_manifest_sha256 = sha256_hex(public_json.as_bytes());
        assert_eq!(controller_commitment(&controller).unwrap(), first);
        assert_eq!(
            controller.public_manifest_sha256,
            sha256_hex(serde_json::to_string_pretty(&public).unwrap().as_bytes())
        );
    }

    #[test]
    #[ignore = "explicit pre-freeze Kotlin compiler smoke; sample seeds are excluded from E04"]
    fn all_family_variant_templates_compile_with_kotlin_2_1() {
        let temporary = tempfile::tempdir().unwrap();
        let repository = temporary.path();
        fs::write(
            repository.join("settings.gradle.kts"),
            "rootProject.name=\"e04-smoke\"\n",
        )
        .unwrap();
        fs::write(
            repository.join("build.gradle.kts"),
            super::super::gradle_module_build(),
        )
        .unwrap();
        let tooling = validate_gradle_wrapper_assets(&repository_wrapper_assets()).unwrap();
        copy_gradle_wrapper(&tooling, repository).unwrap();
        for (family_index, family) in [
            "producer-transform-consumer",
            "type-signature-propagation",
            "dto-event-api-evolution",
            "persistence-nullability",
            "configuration-lifecycle",
            "error-retry-resource",
            "test-regression-strengthening",
        ]
        .into_iter()
        .enumerate()
        {
            for (variant_index, variant) in [
                TaskVariant::Positive,
                TaskVariant::Ambiguous,
                TaskVariant::MustRefuse,
            ]
            .into_iter()
            .enumerate()
            {
                let seed = 10_000 + (family_index * 10 + variant_index) as u64;
                let template = family_template_for(family, variant, seed).unwrap();
                write_dynamic_template(repository, RepositoryLayout::Flat, &template).unwrap();
            }
        }
        let result = Command::new(repository.join("gradlew"))
            .args(["test", "--no-daemon", "--stacktrace"])
            .current_dir(repository)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "Kotlin 2.1 compiler smoke failed:\n{}\n{}",
            String::from_utf8_lossy(&result.stdout),
            String::from_utf8_lossy(&result.stderr)
        );
    }
}
