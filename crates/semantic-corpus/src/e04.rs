//! Post-freeze E04 corpus materialization.
//!
//! This is deliberately an artifact generator. It never invokes a model,
//! binder, source mutation path, or hidden judge.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::population::{self, EditingPopulationSpec, PopulationSlot};
use crate::{
    BuildSystem, GenerateOptions, RepositoryLayout, TaskFamily, TaskVariant, generate_with_variant,
};

pub const E04_PUBLIC_SCHEMA: &str = "semantic-editing-e04-public-task/0.1";
pub const E04_CONTROLLER_SCHEMA: &str = "semantic-editing-e04-controller/0.1";
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
    pub tooling_root: Option<PathBuf>,
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
}

pub fn materialize(options: &MaterializeOptions) -> Result<MaterializedExperiment> {
    if options.binder_freeze != FROZEN_PRODUCT_REVISION
        || options.population_sha256 != FROZEN_POPULATION_SHA256
    {
        bail!("E04 materialization requires the approved binder and population freezes");
    }
    if options.binder_tree_sha256 != FROZEN_BINDER_TREE_SHA256 {
        bail!("E04 requires the canonical frozen binder-tree SHA-256");
    }
    let spec = population::parse_and_validate(&options.population_json)?;
    if spec.slots.len() != 42 {
        bail!("E04 requires exactly 42 frozen slots");
    }
    if options.experiment_root.exists() && options.experiment_root.read_dir()?.next().is_some() {
        bail!("requested experiment root must be absent or empty");
    }
    let agent_root = options.experiment_root.join("agent");
    let controller_root = options.experiment_root.join("controller");
    for slot in &spec.slots {
        materialize_slot(&spec, slot, options, &agent_root, &controller_root)?;
    }
    Ok(MaterializedExperiment {
        agent_root,
        controller_root,
        tasks: spec.slots.len(),
    })
}

fn materialize_slot(
    spec: &EditingPopulationSpec,
    slot: &PopulationSlot,
    options: &MaterializeOptions,
    agent_root: &Path,
    controller_root: &Path,
) -> Result<()> {
    let seed = population::derive_slot_seed(
        spec,
        &options.binder_tree_sha256,
        &options.population_sha256,
        slot,
    )?;
    let slot_key = slot_id(slot);
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
        let tooling = options.tooling_root.as_ref().context("Gradle E04 materialization requires --tooling-root with gradlew and gradle/wrapper assets")?;
        copy_gradle_wrapper(tooling, &generated.agent_dir.join("repository"))?;
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
        ambiguous_choices: (slot.variant == TaskVariant::Ambiguous)
            .then(|| template.ambiguities.clone())
            .unwrap_or_default(),
        refusal_reason: (slot.variant == TaskVariant::MustRefuse).then(|| template.refusal.clone()),
        commitments: vec![
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

fn copy_gradle_wrapper(tooling_root: &Path, repository: &Path) -> Result<()> {
    for relative in [
        "gradlew",
        "gradle/wrapper/gradle-wrapper.jar",
        "gradle/wrapper/gradle-wrapper.properties",
    ] {
        let source = tooling_root.join(relative);
        if !source.is_file() {
            bail!(
                "missing canonical Gradle wrapper asset {}",
                source.display()
            );
        }
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
mod tests {
    use super::*;
    use std::process::Command;
    const SPEC: &str =
        include_str!("../../../benchmarks/semantic-change/editing-population-v1.json");

    #[test]
    fn post_freeze_plan_has_all_slots_but_tests_do_not_materialize_them() {
        let spec = population::parse_and_validate(SPEC).unwrap();
        assert_eq!(spec.slots.len(), 42);
    }

    #[test]
    fn wrong_freeze_is_rejected_before_any_materialization() {
        let root = tempfile::tempdir().unwrap();
        let error = materialize(&MaterializeOptions {
            experiment_root: root.path().join("out"),
            population_json: SPEC.into(),
            binder_freeze: "0".repeat(40),
            binder_tree_sha256: "0".repeat(64),
            population_sha256: FROZEN_POPULATION_SHA256.into(),
            tooling_root: None,
        })
        .unwrap_err();
        assert!(error.to_string().contains("approved binder"));
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
        let tooling = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
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
