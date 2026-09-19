//! Explicit declaration recomposition over immutable source evidence.
use super::{
    bindings, cache,
    check::{self, Check},
    digest, invalid,
    model::Observation,
    processes, review, source_inputs,
    store::{Repository, RepositoryInputs},
};
use crate::error::{ClewError, ErrorCode};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::collections::BTreeMap;

const SCHEMA: &str = "codeclew-documentation-composition/1.0";
const MANIFEST_SCHEMA: &str = "codeclew-documentation-composition-manifest/1.0";
const OPERATION_SCHEMA: &str = "codeclew-documentation-composition-operation/1.0";
const VERSION_SCHEMA: &str = "codeclew-documentation-composition-accepted-version/1.0";
const SCOPE_SCHEMA: &str = "codeclew-documentation-composition-input-scope/1.0";
const MAX_RECORDS: usize = 4096;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Composition {
    pub(super) schema: String,
    pub(super) parent: String,
    pub(super) composer: String,
    pub(super) input_digest: String,
    pub(super) inputs: RepositoryInputs,
    retained: RetainedInputs,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RetainedInputs {
    baseline: Option<bindings::BaselineReceipt>,
    versions: processes::RetainedVersions,
    scopes: BTreeMap<String, Observation>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Manifest {
    schema: String,
    parent: String,
    composer: String,
    input_digest: String,
    inputs: cache::ObjectRef,
    baseline: Option<bindings::BaselineReceipt>,
    operations: BTreeMap<String, cache::ObjectRef>,
    accepted_versions: BTreeMap<String, cache::ObjectRef>,
    scopes: BTreeMap<String, cache::ObjectRef>,
}

fn compatible(original: &RepositoryInputs, current: &RepositoryInputs) -> Result<(), ClewError> {
    if original.services != current.services
        || original.evidence_expectations != current.evidence_expectations
        || original.update_policies != current.update_policies
        || original.update_state != current.update_state
    {
        return Err(invalid(
            "RECOMPOSITION_SOURCE_INPUTS_CHANGED: keep all services, evidence expectations, update policies and targets unchanged; select matching saved evidence or explicitly capture it",
        ));
    }
    Ok(())
}

fn composer() -> Result<String, ClewError> {
    digest(&(
        &[
            SCHEMA,
            include_str!("composition.rs"),
            include_str!("check.rs"),
            include_str!("entities.rs"),
            include_str!("notes.rs"),
            include_str!("dataflow.rs"),
            include_str!("processes.rs"),
            include_str!("review.rs"),
            include_str!("model.rs"),
            include_str!("sections.rs"),
            include_str!("bindings.rs"),
            include_str!("work.rs"),
        ],
        super::dataflow::module()?,
    ))
}

/// Compare immutable source references, not hydrated copies of all source
/// payloads. v1 requires a direct original capture parent and has no recursive
/// derivative chain. Keep that parent manifest as a retention dependency.
pub(super) fn validate_parent_manifest(
    repo: &Repository,
    value: &Composition,
    derived: &check::CheckManifest,
    original_input_digest: &str,
) -> Result<(), ClewError> {
    let parent = Check::load_snapshot_manifest(repo, &value.parent)?;
    if parent.input_digest != original_input_digest
        || parent.composition.is_some()
        || parent.source_inputs.is_none()
        || parent.source_inputs != derived.source_inputs
        || parent.unresolved != derived.unresolved
        || digest(&parent.service_manifests)? != digest(&derived.service_manifests)?
    {
        return Err(invalid(
            "composition source evidence does not match its original capture parent",
        ));
    }
    Ok(())
}

fn capture_retained(
    repo: &Repository,
    inputs: &RepositoryInputs,
) -> Result<RetainedInputs, ClewError> {
    let baseline = bindings::capture_baseline(repo)?;
    let versions = baseline
        .as_ref()
        .map(|(_, b)| processes::projection(&inputs.scenarios, b))
        .unwrap_or_else(|| processes::RetainedVersions {
            operations: BTreeMap::new(),
            accepted_versions: BTreeMap::new(),
        });
    if versions.accepted_versions.len() > MAX_RECORDS || versions.operations.len() > MAX_RECORDS {
        return Err(invalid("retained composition exceeds its record budget"));
    }
    let scopes = review::capture_scopes(
        repo,
        versions.accepted_versions.values().cloned(),
        &inputs.notes,
    )?;
    Ok(RetainedInputs {
        baseline: baseline.map(|(receipt, _)| receipt),
        versions,
        scopes,
    })
}

fn attach(
    inputs: &RepositoryInputs,
    retained: &RetainedInputs,
    checked: &mut Check,
) -> Result<(), ClewError> {
    check::attach_catalogue_from_inputs(inputs, checked)?;
    review::attach_scopes(checked, &retained.scopes)?;
    processes::attach_versions_from_inputs(
        &inputs.scenarios,
        &inputs.interactions,
        checked,
        Some(&retained.versions),
    )
}

pub(super) fn attach_retained(
    repo: &Repository,
    inputs: &RepositoryInputs,
    checked: &mut Check,
) -> Result<(), ClewError> {
    let retained = capture_retained(repo, inputs)?;
    review::attach_scopes(checked, &retained.scopes)?;
    processes::attach_versions_from_inputs(
        &inputs.scenarios,
        &inputs.interactions,
        checked,
        Some(&retained.versions),
    )
}

/// A new immutable result, never an update to latest, accepted prose or source.
/// v1 intentionally starts from an original capture, not a chain of derivatives.
pub fn recompose(repo: &Repository, parent: &str) -> Result<(Check, String), ClewError> {
    let original = Check::load_snapshot(repo, parent)?;
    if original.composition.is_some() {
        return Err(invalid(
            "RECOMPOSITION_REQUIRES_CAPTURE_PARENT: use the original source-capture snapshot",
        ));
    }
    let source = original.source_inputs.as_ref().ok_or_else(|| invalid("RECOMPOSITION_INPUTS_UNAVAILABLE: legacy snapshot has no consumed source-input contract"))?;
    let inputs = repo.inputs()?;
    compatible(&source.inputs, &inputs)?;
    let input_digest = digest(&inputs)?;
    let retained = capture_retained(repo, &inputs)?;
    let mut checked = check::assemble(
        input_digest.clone(),
        original.services,
        original.unresolved,
        &inputs.interactions,
        &inputs.scenarios,
    )?;
    attach(&inputs, &retained, &mut checked)?;
    checked.source_inputs = original.source_inputs;
    checked.composition = Some(Composition {
        schema: SCHEMA.into(),
        parent: parent.into(),
        composer: composer()?,
        input_digest,
        inputs,
        retained,
    });
    // Detect a currently different documentation bundle without claiming an
    // atomic filesystem snapshot. All computation above consumed owned values.
    let _lock = repo.lock()?;
    ensure_current(repo, checked.composition.as_ref().unwrap())?;
    let handle = checked.store_snapshot(repo)?.0;
    Ok((checked, handle))
}

fn ensure_current(repo: &Repository, composition: &Composition) -> Result<(), ClewError> {
    if composition.input_digest != repo.input_digest()? {
        return Err(invalid("documentation input changed during recomposition"));
    }
    if digest(&capture_retained(repo, &composition.inputs)?)? != digest(&composition.retained)? {
        return Err(invalid(
            "retained documentation inputs changed during recomposition",
        ));
    }
    Ok(())
}

pub(super) fn validate(
    value: &Composition,
    source: &check::SourceInputs,
    input_digest: &str,
) -> Result<(), ClewError> {
    if value.schema != SCHEMA || value.input_digest != input_digest {
        return Err(invalid("saved composition input contract is invalid"));
    }
    let (parent_digest, size) = value
        .parent
        .rsplit_once('/')
        .ok_or_else(|| invalid("composition parent is not an immutable handle"))?;
    let size: u64 = size
        .parse()
        .map_err(|_| invalid("invalid composition parent size"))?;
    if value.parent != format!("{parent_digest}/{size}")
        || !valid_digest(parent_digest)
        || !valid_digest(&value.composer)
    {
        return Err(invalid("invalid composition identity"));
    }
    compatible(&source.inputs, &value.inputs)?;
    if value.retained.baseline.is_none()
        && (!value.retained.versions.operations.is_empty()
            || !value.retained.versions.accepted_versions.is_empty()
            || !value.retained.scopes.is_empty())
    {
        return Err(invalid(
            "retained composition has records without a baseline",
        ));
    }
    for (id, observation) in &value.retained.scopes {
        if id != &observation.id
            || observation.kind != "DOCUMENTATION_INPUT_SCOPE"
            || digest(&observation.normalized)? != observation.digest
        {
            return Err(invalid("retained composition input scope is invalid"));
        }
    }
    Ok(())
}

fn valid_digest(value: &str) -> bool {
    value.strip_prefix("sha256:").is_some_and(|v| {
        v.len() == 64
            && v.bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
    })
}

fn store_map<T: Serialize>(
    repo: &Repository,
    schema: &str,
    values: &BTreeMap<String, T>,
) -> Result<BTreeMap<String, cache::ObjectRef>, ClewError> {
    if values.len() > MAX_RECORDS {
        return Err(invalid("composition map exceeds its record budget"));
    }
    values
        .iter()
        .map(|(id, v)| Ok((id.clone(), cache::put_json(repo, schema, v)?)))
        .collect()
}
fn load_map<T: DeserializeOwned>(
    repo: &Repository,
    schema: &str,
    values: BTreeMap<String, cache::ObjectRef>,
) -> Result<BTreeMap<String, T>, ClewError> {
    if values.len() > MAX_RECORDS {
        return Err(invalid("composition map exceeds its record budget"));
    }
    values
        .into_iter()
        .map(|(id, r)| {
            if r.schema != schema {
                return Err(invalid("composition field schema mismatch"));
            }
            let value =
                cache::get_json(repo, &r, check::PORTABLE_CACHE_MAX_BYTES)?.ok_or_else(|| {
                    ClewError::new(ErrorCode::StateCorrupt, "composition object is missing")
                })?;
            Ok((id, value))
        })
        .collect()
}

pub(super) fn store(repo: &Repository, value: &Composition) -> Result<cache::ObjectRef, ClewError> {
    let manifest = Manifest {
        schema: MANIFEST_SCHEMA.into(),
        parent: value.parent.clone(),
        composer: value.composer.clone(),
        input_digest: value.input_digest.clone(),
        inputs: source_inputs::store_declarations(repo, &value.inputs, &value.input_digest)?,
        baseline: value.retained.baseline.clone(),
        operations: store_map(repo, OPERATION_SCHEMA, &value.retained.versions.operations)?,
        accepted_versions: store_map(
            repo,
            VERSION_SCHEMA,
            &value.retained.versions.accepted_versions,
        )?,
        scopes: store_map(repo, SCOPE_SCHEMA, &value.retained.scopes)?,
    };
    cache::put_json(repo, MANIFEST_SCHEMA, &manifest)
}

pub(super) fn load(
    repo: &Repository,
    reference: &cache::ObjectRef,
) -> Result<Composition, ClewError> {
    if reference.schema != MANIFEST_SCHEMA {
        return Err(invalid("composition manifest schema mismatch"));
    }
    let m: Manifest = cache::get_json(repo, reference, check::PORTABLE_CACHE_MAX_BYTES)?
        .ok_or_else(|| {
            ClewError::new(ErrorCode::StateCorrupt, "composition manifest is missing")
        })?;
    if m.schema != MANIFEST_SCHEMA {
        return Err(invalid("unsupported composition manifest schema"));
    }
    let (inputs, input_digest) = source_inputs::load_declarations(repo, &m.inputs)?;
    if input_digest != m.input_digest {
        return Err(invalid("composition declaration digest mismatch"));
    }
    Ok(Composition {
        schema: SCHEMA.into(),
        parent: m.parent,
        composer: m.composer,
        input_digest: m.input_digest,
        inputs,
        retained: RetainedInputs {
            baseline: m.baseline,
            versions: processes::RetainedVersions {
                operations: load_map(repo, OPERATION_SCHEMA, m.operations)?,
                accepted_versions: load_map(repo, VERSION_SCHEMA, m.accepted_versions)?,
            },
            scopes: load_map(repo, SCOPE_SCHEMA, m.scopes)?,
        },
    })
}

#[cfg(test)]
#[path = "composition_tests.rs"]
mod tests;
