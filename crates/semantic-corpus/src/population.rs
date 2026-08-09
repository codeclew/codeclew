//! Executable D02 contract for the future editing corpus.
//!
//! This module validates a population plan. It deliberately does not expose
//! final seeds or generate evaluation instances before the binder freeze.

use std::collections::BTreeSet;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{BuildSystem, TaskVariant};

pub const EDITING_POPULATION_SCHEMA: &str = "codeclew-editing-population/0.1";
pub const EDITING_GENERATOR_PROTOCOL: &str = "semantic-editing-corpus/0.1";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct EditingPopulationSpec {
    pub schema: String,
    pub outcome: PopulationOutcome,
    pub generator_protocol: String,
    pub planned_task_count: u32,
    pub families: Vec<FamilySpec>,
    pub slots: Vec<PopulationSlot>,
    pub seed_derivation: SeedDerivation,
    pub ecology: EcologyBoundary,
    pub annotation_protocol: AnnotationProtocol,
    pub isolation: IsolationRules,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum PopulationOutcome {
    NarrowPopulation,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct FamilySpec {
    pub id: String,
    pub planned_instances: u32,
    pub variants: Vec<TaskVariant>,
    pub build_systems: Vec<BuildSystem>,
    pub required_obligations: Vec<String>,
    pub must_refuse_boundaries: Vec<String>,
    pub decoy_dimensions: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct PopulationSlot {
    pub family: String,
    pub variant: TaskVariant,
    pub build_system: BuildSystem,
    pub ordinal: u32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MaterializedTaskIdentity {
    pub family: String,
    pub variant: TaskVariant,
    pub build_system: BuildSystem,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct SeedDerivation {
    pub materialization_node: String,
    pub exact_instances_materialized: bool,
    pub algorithm: String,
    pub domain: String,
    pub binder_digest_input: String,
    pub population_digest_input: String,
    pub variant_assignment: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct EcologyBoundary {
    pub status: EcologyStatus,
    pub weighting: String,
    pub typical_task_claim_allowed: bool,
    pub limitation: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum EcologyStatus {
    Unavailable,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct AnnotationProtocol {
    pub annotator_count: u8,
    pub blinded_to_arm_results: bool,
    pub labels: Vec<String>,
    pub disagreement_rule: AdjudicationRule,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum AdjudicationRule {
    ThirdAnnotatorBeforeArmExecutionRetainOriginals,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct IsolationRules {
    pub final_instances_visible_before_binder_freeze: bool,
    pub worker_may_depend_on_generator: bool,
    pub public_package_may_contain_oracle: bool,
    pub public_samples_excluded_from_evaluation: bool,
}

pub fn parse_and_validate(json: &str) -> Result<EditingPopulationSpec> {
    let spec: EditingPopulationSpec =
        serde_json::from_str(json).context("parse editing population specification")?;
    validate(&spec)?;
    Ok(spec)
}

pub fn validate(spec: &EditingPopulationSpec) -> Result<()> {
    if spec.schema != EDITING_POPULATION_SCHEMA
        || spec.outcome != PopulationOutcome::NarrowPopulation
        || spec.generator_protocol != EDITING_GENERATOR_PROTOCOL
    {
        bail!("unsupported editing population identity");
    }
    if spec.families.len() < 6 {
        bail!("editing population requires at least six structural families");
    }
    let family_ids = spec
        .families
        .iter()
        .map(|family| family.id.as_str())
        .collect::<BTreeSet<_>>();
    if family_ids.len() != spec.families.len() || family_ids.iter().any(|id| id.trim().is_empty()) {
        bail!("editing family ids must be unique and nonempty");
    }

    let all_variants = BTreeSet::from([
        "positive".to_owned(),
        "ambiguous".to_owned(),
        "must-refuse".to_owned(),
    ]);
    let all_builds = BTreeSet::from(["gradle".to_owned(), "maven".to_owned()]);
    let mut total = 0_u32;
    for family in &spec.families {
        let variants = family
            .variants
            .iter()
            .map(ToString::to_string)
            .collect::<BTreeSet<_>>();
        let builds = family
            .build_systems
            .iter()
            .map(ToString::to_string)
            .collect::<BTreeSet<_>>();
        let matrix_size = variants.len() * builds.len();
        if variants != all_variants
            || builds != all_builds
            || family.planned_instances < matrix_size as u32
        {
            bail!("every editing family must cover all variant/build combinations");
        }
        if family.required_obligations.is_empty()
            || family.must_refuse_boundaries.is_empty()
            || family.decoy_dimensions.is_empty()
        {
            bail!("every editing family needs obligations, refusals and decoys");
        }
        total = total
            .checked_add(family.planned_instances)
            .context("planned task count overflow")?;
    }
    if total != spec.planned_task_count || total < 36 {
        bail!("planned task count must be exact and at least 36");
    }

    validate_slots(spec)?;

    if spec.seed_derivation.materialization_node != "E04"
        || spec.seed_derivation.exact_instances_materialized
        || spec.seed_derivation.algorithm != "SHA256_V1_U64_BE_PREFIX"
        || spec.seed_derivation.domain != "codeclew-e04-v1"
        || spec.seed_derivation.binder_digest_input != "binder-source-tree-sha256"
        || spec.seed_derivation.population_digest_input != "population-spec-sha256"
        || spec.seed_derivation.variant_assignment != "FROZEN_SLOT"
    {
        bail!("final seeds must be derived only after the E04 binder freeze");
    }
    if spec.ecology.status != EcologyStatus::Unavailable
        || spec.ecology.typical_task_claim_allowed
        || spec.ecology.weighting != "BALANCED_STRUCTURAL_SAFETY"
        || spec.ecology.limitation.trim().is_empty()
    {
        bail!("unavailable ecology must forbid typical-task claims");
    }
    if spec.annotation_protocol.annotator_count < 2
        || !spec.annotation_protocol.blinded_to_arm_results
        || spec.annotation_protocol.disagreement_rule
            != AdjudicationRule::ThirdAnnotatorBeforeArmExecutionRetainOriginals
    {
        bail!("annotation protocol must be blinded and independently adjudicated");
    }
    let required_labels = BTreeSet::from([
        "family",
        "variant",
        "required-obligations",
        "must-refuse-reason",
        "acceptable-design-class",
    ]);
    if spec
        .annotation_protocol
        .labels
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>()
        != required_labels
    {
        bail!("annotation protocol is missing required semantic labels");
    }
    if spec.isolation.final_instances_visible_before_binder_freeze
        || spec.isolation.worker_may_depend_on_generator
        || spec.isolation.public_package_may_contain_oracle
        || !spec.isolation.public_samples_excluded_from_evaluation
    {
        bail!("population isolation contract is unsafe");
    }
    Ok(())
}

fn validate_slots(spec: &EditingPopulationSpec) -> Result<()> {
    if spec.slots.len() != spec.planned_task_count as usize {
        bail!("population slots must enumerate every planned instance");
    }
    let unique = spec.slots.iter().cloned().collect::<BTreeSet<_>>();
    if unique.len() != spec.slots.len() {
        bail!("population slots must be unique");
    }
    for family in &spec.families {
        let slots = spec
            .slots
            .iter()
            .filter(|slot| slot.family == family.id)
            .collect::<Vec<_>>();
        if slots.len() != family.planned_instances as usize {
            bail!("family slot count does not match its plan");
        }
        for variant in &family.variants {
            for build_system in &family.build_systems {
                if !slots.iter().any(|slot| {
                    slot.variant == *variant
                        && slot.build_system == *build_system
                        && slot.ordinal == 0
                }) {
                    bail!("family slot matrix is missing a variant/build cell");
                }
            }
        }
    }
    if spec
        .slots
        .iter()
        .any(|slot| !spec.families.iter().any(|family| family.id == slot.family))
    {
        bail!("population slot names an unknown family");
    }
    Ok(())
}

pub fn derive_slot_seed(
    spec: &EditingPopulationSpec,
    binder_source_tree_sha256: &str,
    population_spec_sha256: &str,
    slot: &PopulationSlot,
) -> Result<u64> {
    validate(spec)?;
    for (name, digest) in [
        ("binder source tree", binder_source_tree_sha256),
        ("population specification", population_spec_sha256),
    ] {
        if digest.len() != 64 || !digest.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            bail!("{name} digest must be SHA-256 hex");
        }
    }
    if !spec.slots.contains(slot) {
        bail!("cannot derive a seed for an unregistered population slot");
    }
    let payload = serde_json::json!({
        "domain": spec.seed_derivation.domain,
        "binderSourceTreeSha256": binder_source_tree_sha256,
        "populationSpecSha256": population_spec_sha256,
        "slot": slot,
    });
    let canonical = serde_json::to_vec(&payload).context("serialize seed payload")?;
    let digest = Sha256::digest(canonical);
    Ok(u64::from_be_bytes(
        digest[..8]
            .try_into()
            .expect("SHA-256 prefix has eight bytes"),
    ))
}

pub fn validate_materialized_identity(
    slot: &PopulationSlot,
    actual: &MaterializedTaskIdentity,
) -> Result<()> {
    if slot.family != actual.family
        || slot.variant != actual.variant
        || slot.build_system != actual.build_system
    {
        bail!("materialized task does not match its frozen population slot");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SPEC: &str =
        include_str!("../../../benchmarks/semantic-change/editing-population-v1.json");

    #[test]
    fn frozen_population_is_executable_and_narrow() {
        let spec = parse_and_validate(SPEC).unwrap();
        assert_eq!(spec.planned_task_count, 42);
        assert_eq!(spec.families.len(), 7);
        assert_eq!(spec.slots.len(), 42);
        assert!(!spec.ecology.typical_task_claim_allowed);
        assert!(!spec.seed_derivation.exact_instances_materialized);
    }

    #[test]
    fn missing_variant_and_premature_seed_reveal_fail_closed() {
        let mut spec = parse_and_validate(SPEC).unwrap();
        spec.families[0].variants.pop();
        assert!(validate(&spec).is_err());

        let mut spec = parse_and_validate(SPEC).unwrap();
        spec.seed_derivation.exact_instances_materialized = true;
        assert!(validate(&spec).is_err());
    }

    #[test]
    fn every_slot_has_a_deterministic_seed_and_must_match_materialization() {
        let spec = parse_and_validate(SPEC).unwrap();
        let slot = &spec.slots[0];
        let digest = "a".repeat(64);
        let first = derive_slot_seed(&spec, &digest, &digest, slot).unwrap();
        let second = derive_slot_seed(&spec, &digest, &digest, slot).unwrap();
        assert_eq!(first, second);
        let identity = MaterializedTaskIdentity {
            family: slot.family.clone(),
            variant: slot.variant,
            build_system: slot.build_system,
        };
        validate_materialized_identity(slot, &identity).unwrap();

        let mut wrong = identity;
        wrong.variant = match slot.variant {
            TaskVariant::Positive => TaskVariant::Ambiguous,
            _ => TaskVariant::Positive,
        };
        assert!(validate_materialized_identity(slot, &wrong).is_err());
    }

    #[test]
    fn duplicate_or_missing_slots_fail_closed() {
        let mut spec = parse_and_validate(SPEC).unwrap();
        spec.slots[1] = spec.slots[0].clone();
        assert!(validate(&spec).is_err());

        let mut spec = parse_and_validate(SPEC).unwrap();
        spec.slots.pop();
        assert!(validate(&spec).is_err());
    }

    #[test]
    fn ecology_cannot_be_laundered_into_a_typical_task_claim() {
        let mut spec = parse_and_validate(SPEC).unwrap();
        spec.ecology.typical_task_claim_allowed = true;
        assert!(validate(&spec).is_err());
    }

    #[test]
    fn product_worker_does_not_depend_on_corpus_generator() {
        let product_manifest = include_str!("../../sthread/Cargo.toml");
        assert!(!product_manifest.contains("semantic-corpus"));
    }
}
