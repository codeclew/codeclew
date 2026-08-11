//! Controller-free preregistration of the positive E04 product-coverage ceiling.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::BuildSystem;
use crate::population;

pub const PRODUCT_COVERAGE_SCHEMA: &str = "semantic-editing-e04-product-coverage-contract/0.1";
pub const PRODUCT_COVERAGE_REPORT_SCHEMA: &str = "semantic-editing-e04-product-coverage/0.1";
const CONTRACT_JSON: &str =
    include_str!("../../../benchmarks/semantic-change/e04-product-coverage-v1.json");
const POPULATION_JSON: &str =
    include_str!("../../../benchmarks/semantic-change/editing-population-v1.json");
const MAX_TYPED_GOAL_CATALOG_BYTES: u64 = 256 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ProductCoverageContract {
    pub schema: String,
    pub population_schema: String,
    pub population_sha256: String,
    pub typed_goal_schema: String,
    pub typed_goal_version: String,
    pub positive_cell_count: usize,
    pub current_supported_upper_bound: usize,
    pub cells: Vec<ProductCoverageCell>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ProductCoverageCell {
    pub family: String,
    pub build_system: BuildSystem,
    pub required_roles: Vec<String>,
    pub required_obligations: Vec<String>,
    pub required_root: Option<CoverageRoot>,
    pub expected_provider_binding_cardinality: usize,
    pub status: CoverageStatus,
    pub unsupported_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct CoverageRoot {
    pub operator: String,
    pub operand_roles: Vec<String>,
    pub composition_source: CompositionSource,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum CompositionSource {
    TypedGoalMandatoryClosure,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum CoverageStatus {
    Supported,
    IncompleteRequiredRoleClosure,
    Unsupported,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoverageSummary {
    pub positive_cells: usize,
    pub supported_upper_bound: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ProductCoverageReport {
    pub schema: String,
    pub contract_sha256: String,
    pub population_sha256: String,
    pub catalog_sha256: String,
    pub positive_cells: usize,
    pub supported_upper_bound: usize,
    pub cell_results: Vec<ProductCoverageCell>,
}

struct RegistryOperator {
    domain: String,
    arity: usize,
    auxiliary_only: bool,
}

/// Validate the frozen 14-cell contract against the frozen population and the
/// product's machine-readable typed-goal language. No controller or task result
/// is an input, so this computes a capability ceiling rather than benchmark success.
pub fn validate_product_coverage(
    contract_json: &str,
    population_json: &str,
    typed_goal_registry: &Value,
) -> Result<CoverageSummary> {
    let contract: ProductCoverageContract =
        serde_json::from_str(contract_json).context("parse product coverage contract")?;
    let population = population::parse_and_validate(population_json)?;
    let population_sha256 = hex::encode(Sha256::digest(population_json.as_bytes()));
    if contract.schema != PRODUCT_COVERAGE_SCHEMA
        || contract.population_schema != population.schema
        || contract.population_sha256 != population_sha256
        || contract.typed_goal_schema != typed_goal_registry["schema"].as_str().unwrap_or_default()
        || contract.typed_goal_version
            != typed_goal_registry["version"].as_str().unwrap_or_default()
    {
        bail!("product coverage contract identity does not match frozen inputs");
    }
    let executable_domains = typed_goal_registry["executableDomains"]
        .as_array()
        .context("typed-goal registry lacks executableDomains")?
        .iter()
        .map(|value| {
            value
                .as_str()
                .map(str::to_owned)
                .context("typed-goal executable domain is not a string")
        })
        .collect::<Result<BTreeSet<_>>>()?;
    let operators = typed_goal_registry["operators"]
        .as_array()
        .context("typed-goal registry lacks operators")?
        .iter()
        .map(|value| {
            let operator = value["operator"]
                .as_str()
                .context("typed-goal operator lacks identity")?
                .to_owned();
            let entry = RegistryOperator {
                domain: value["constraintDomain"]
                    .as_str()
                    .context("typed-goal operator lacks constraint domain")?
                    .to_owned(),
                arity: value["arity"]
                    .as_u64()
                    .context("typed-goal operator lacks arity")?
                    .try_into()
                    .context("typed-goal arity overflows usize")?,
                auxiliary_only: value["auxiliaryOnly"]
                    .as_bool()
                    .context("typed-goal operator lacks auxiliaryOnly")?,
            };
            Ok((operator, entry))
        })
        .collect::<Result<BTreeMap<_, _>>>()?;
    if operators.len()
        != typed_goal_registry["operators"]
            .as_array()
            .expect("checked above")
            .len()
    {
        bail!("typed-goal registry repeats an operator identity");
    }

    let expected_cells = population
        .families
        .iter()
        .flat_map(|family| {
            family
                .build_systems
                .iter()
                .map(move |build| (family, *build))
        })
        .collect::<Vec<_>>();
    if contract.positive_cell_count != expected_cells.len()
        || contract.cells.len() != expected_cells.len()
        || expected_cells.len() != 14
    {
        bail!("product coverage contract must enumerate the exact 14 positive cells");
    }

    let mut supported = 0;
    for (cell, (family, build_system)) in contract.cells.iter().zip(expected_cells) {
        if cell.family != family.id
            || cell.build_system != build_system
            || cell.required_obligations != family.required_obligations
            || cell.required_roles.len() != 3
            || cell.required_roles.iter().collect::<BTreeSet<_>>().len() != 3
        {
            bail!("coverage cell differs from the frozen family/build contract");
        }
        let executable = if let Some(root) = &cell.required_root {
            if root.composition_source != CompositionSource::TypedGoalMandatoryClosure
                || root.operand_roles.iter().collect::<BTreeSet<_>>().len()
                    != root.operand_roles.len()
                || !root
                    .operand_roles
                    .iter()
                    .all(|role| cell.required_roles.contains(role))
            {
                bail!("coverage root is not a canonical subset of required roles");
            }
            let entry = operators.get(&root.operator);
            let actual_cardinality = entry.map_or(0, |entry| entry.arity);
            if cell.expected_provider_binding_cardinality != actual_cardinality {
                bail!("coverage cell provider cardinality differs from typed-goal registry");
            }
            entry.is_some_and(|entry| {
                !entry.auxiliary_only
                    && executable_domains.contains(&entry.domain)
                    && entry.arity == root.operand_roles.len()
                    && cell.expected_provider_binding_cardinality == cell.required_roles.len()
                    && root.operand_roles.iter().collect::<BTreeSet<_>>()
                        == cell.required_roles.iter().collect::<BTreeSet<_>>()
            })
        } else {
            if cell.expected_provider_binding_cardinality != 0 {
                bail!("coverage cell without a root cannot claim provider bindings");
            }
            false
        };
        if executable {
            supported += 1;
            if cell.status != CoverageStatus::Supported || cell.unsupported_reason.is_some() {
                bail!("executable coverage cell is not declared SUPPORTED");
            }
        } else if cell.status == CoverageStatus::Supported
            || cell.unsupported_reason.as_deref().is_none_or(str::is_empty)
        {
            bail!("non-executable coverage cell lacks an exact unsupported status/reason");
        }
    }
    if supported != contract.current_supported_upper_bound {
        bail!("declared product coverage upper bound does not recompute");
    }
    Ok(CoverageSummary {
        positive_cells: contract.cells.len(),
        supported_upper_bound: supported,
    })
}

/// Build the exact controller-free report consumed by the readiness issuer.
/// The contract and population are compiled into the binary; callers can only
/// provide the product's canonical machine-readable language catalog.
pub fn product_coverage_report(typed_goal_catalog: &Path) -> Result<ProductCoverageReport> {
    let metadata =
        fs::symlink_metadata(typed_goal_catalog).context("typed-goal catalog is missing")?;
    if metadata.file_type().is_symlink()
        || !metadata.is_file()
        || metadata.len() > MAX_TYPED_GOAL_CATALOG_BYTES
    {
        bail!("typed-goal catalog must be a bounded regular non-symlink file");
    }
    let bytes = fs::read(typed_goal_catalog)?;
    let catalog: Value =
        serde_json::from_slice(&bytes).context("typed-goal catalog is invalid JSON")?;
    validate_catalog_shape(&catalog)?;
    let canonical = crate::e04::canonical_json(&catalog)?;
    if bytes != canonical.as_bytes() {
        bail!("typed-goal catalog is not exact canonical JSON");
    }
    let summary = validate_product_coverage(CONTRACT_JSON, POPULATION_JSON, &catalog)?;
    let contract: ProductCoverageContract = serde_json::from_str(CONTRACT_JSON)
        .context("compiled product coverage contract is invalid")?;
    Ok(ProductCoverageReport {
        schema: PRODUCT_COVERAGE_REPORT_SCHEMA.into(),
        contract_sha256: sha256(CONTRACT_JSON.as_bytes()),
        population_sha256: sha256(POPULATION_JSON.as_bytes()),
        catalog_sha256: sha256(&bytes),
        positive_cells: summary.positive_cells,
        supported_upper_bound: summary.supported_upper_bound,
        cell_results: contract.cells,
    })
}

fn validate_catalog_shape(catalog: &Value) -> Result<()> {
    let object = catalog
        .as_object()
        .context("typed-goal catalog root must be an object")?;
    let expected = BTreeSet::from([
        "schema",
        "version",
        "requestSchema",
        "goalSchema",
        "decisionSchema",
        "maxRequestBytes",
        "variableDomains",
        "executableDomains",
        "productRefusalReasons",
        "operators",
    ]);
    if object.keys().map(String::as_str).collect::<BTreeSet<_>>() != expected {
        bail!("typed-goal catalog top-level shape is not exact");
    }
    for operator in object["operators"]
        .as_array()
        .context("typed-goal operators must be an array")?
    {
        let operator = operator
            .as_object()
            .context("typed-goal operator must be an object")?;
        let expected = BTreeSet::from([
            "operator",
            "constraintDomain",
            "arity",
            "operandDomains",
            "auxiliaryOnly",
            "refusalOnUnknown",
            "requiredEvidenceRelations",
            "mandatoryApplications",
        ]);
        if operator.keys().map(String::as_str).collect::<BTreeSet<_>>() != expected {
            bail!("typed-goal operator shape is not exact");
        }
        for application in operator["mandatoryApplications"]
            .as_array()
            .context("typed-goal mandatory applications must be an array")?
        {
            let application = application
                .as_object()
                .context("typed-goal mandatory application must be an object")?;
            if application
                .keys()
                .map(String::as_str)
                .collect::<BTreeSet<_>>()
                != BTreeSet::from(["operator", "operandIndices"])
            {
                bail!("typed-goal mandatory application shape is not exact");
            }
        }
    }
    Ok(())
}

fn sha256(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    const CONTRACT: &str =
        include_str!("../../../benchmarks/semantic-change/e04-product-coverage-v1.json");
    const POPULATION: &str =
        include_str!("../../../benchmarks/semantic-change/editing-population-v1.json");

    fn registry() -> Value {
        serde_json::to_value(clew::semantic_goal::typed_goal_language_schema()).unwrap()
    }

    fn write_catalog(directory: &Path, catalog: &Value) -> std::path::PathBuf {
        let path = directory.join("typed-goal-catalog.json");
        fs::write(&path, crate::e04::canonical_json(catalog).unwrap()).unwrap();
        path
    }

    #[test]
    fn product_coverage_contract_recomputes_two_of_fourteen() {
        let summary = validate_product_coverage(CONTRACT, POPULATION, &registry()).unwrap();
        assert_eq!(summary.positive_cells, 14);
        assert_eq!(summary.supported_upper_bound, 2);

        let contract: ProductCoverageContract = serde_json::from_str(CONTRACT).unwrap();
        let type_cells = contract
            .cells
            .iter()
            .filter(|cell| cell.family == "type-signature-propagation")
            .collect::<Vec<_>>();
        assert_eq!(type_cells.len(), 2);
        assert!(type_cells.iter().all(|cell| {
            cell.status == CoverageStatus::IncompleteRequiredRoleClosure
                && cell.expected_provider_binding_cardinality == 2
                && cell.required_roles.len() == 3
        }));
        assert_eq!(
            contract
                .cells
                .iter()
                .filter(|cell| cell.status == CoverageStatus::Unsupported)
                .count(),
            10
        );
    }

    #[test]
    fn coverage_checker_positive_control_can_recognize_at_least_nine_cells() {
        let mut contract: ProductCoverageContract = serde_json::from_str(CONTRACT).unwrap();
        for cell in contract.cells.iter_mut().take(10) {
            cell.required_root = Some(CoverageRoot {
                operator: "MAP_EDGE".into(),
                operand_roles: cell.required_roles.clone(),
                composition_source: CompositionSource::TypedGoalMandatoryClosure,
            });
            cell.expected_provider_binding_cardinality = 3;
            cell.status = CoverageStatus::Supported;
            cell.unsupported_reason = None;
        }
        contract.current_supported_upper_bound = 10;
        let synthetic = serde_json::to_string(&contract).unwrap();
        let summary = validate_product_coverage(&synthetic, POPULATION, &registry()).unwrap();
        assert_eq!(summary.supported_upper_bound, 10);
    }

    #[test]
    fn coverage_contract_rejects_role_cardinality_and_registry_drift() {
        let mut contract: Value = serde_json::from_str(CONTRACT).unwrap();
        contract["cells"][2]["requiredRoles"] = serde_json::json!(["DECLARATION", "OVERRIDE"]);
        assert!(
            validate_product_coverage(
                &serde_json::to_string(&contract).unwrap(),
                POPULATION,
                &registry()
            )
            .is_err()
        );

        let mut changed_registry = registry();
        changed_registry["executableDomains"] = serde_json::json!(["DECLARATION_CHANGE"]);
        assert!(validate_product_coverage(CONTRACT, POPULATION, &changed_registry).is_err());
    }

    #[test]
    fn product_coverage_cli_input_is_canonical_pinned_and_controller_free() {
        let temporary = tempfile::tempdir().unwrap();
        let path = write_catalog(temporary.path(), &registry());
        let report = product_coverage_report(&path).unwrap();
        assert_eq!(report.schema, PRODUCT_COVERAGE_REPORT_SCHEMA);
        assert_eq!(report.contract_sha256, sha256(CONTRACT.as_bytes()));
        assert_eq!(report.population_sha256, sha256(POPULATION.as_bytes()));
        assert_eq!(report.catalog_sha256, sha256(&fs::read(&path).unwrap()));
        assert_eq!(report.positive_cells, 14);
        assert_eq!(report.supported_upper_bound, 2);
        assert_eq!(report.cell_results.len(), 14);

        let mut changed = registry();
        changed["executableDomains"] = serde_json::json!(["DECLARATION_CHANGE"]);
        let changed_path = write_catalog(temporary.path(), &changed);
        assert!(product_coverage_report(&changed_path).is_err());

        fs::write(
            &changed_path,
            format!(" {}", crate::e04::canonical_json(&registry()).unwrap()),
        )
        .unwrap();
        assert!(product_coverage_report(&changed_path).is_err());

        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;
            let actual = write_catalog(temporary.path(), &registry());
            let linked = temporary.path().join("linked-catalog.json");
            symlink(actual, &linked).unwrap();
            assert!(product_coverage_report(&linked).is_err());
        }
    }
}
