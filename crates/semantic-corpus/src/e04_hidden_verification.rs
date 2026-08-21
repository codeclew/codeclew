//! Signed, fail-closed verification authority for fresh R1 hidden packages.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Component, Path, PathBuf};

use anyhow::{Context, Result, bail};
use ed25519_dalek::VerifyingKey;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::e04::{
    AgentPublicMember, ControllerMember, ControllerTask, E04MemberSets, ExpectedOutcome,
    canonical_json, inspect_materialized_member_sets,
};
use crate::e04_authorization::{
    R1_AUTHORIZATION_ISSUER, canonical_absent_output, canonical_directory, contained_content_file,
    content_address, pinned_readiness_contract, production_verifying_key, read_canonical_json,
    verify_pinned_readiness_root, verify_purpose_signature,
};

pub const R1_HIDDEN_AUTHORIZATION_ENVELOPE_SCHEMA: &str =
    "semantic-editing-e04-r1-hidden-verification-authorization-envelope/0.1";
pub const R1_HIDDEN_AUTHORIZATION_SCHEMA: &str =
    "semantic-editing-e04-r1-hidden-verification-authorization/0.1";
pub const R1_HIDDEN_VERIFICATION_REPORT_SCHEMA: &str =
    "semantic-editing-e04-r1-hidden-verification-report/0.1";
pub const R1_HIDDEN_VERIFICATION_PURPOSE: &str = "codeclew/e04/r1-hidden-verify/0.1";
pub const R1_HIDDEN_ROOT: &str = "R1_HIDDEN_VERIFY_START_READY";
pub const R1_BLIND_ANNOTATION_SCHEMA: &str = "semantic-editing-e04-r1-blind-annotation/0.1";
pub const R1_BLIND_ANNOTATOR_A: &str = "R1_BLIND_ANNOTATOR_A";
pub const R1_BLIND_ANNOTATOR_B: &str = "R1_BLIND_ANNOTATOR_B";

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct HiddenAuthorizationPayload {
    schema: String,
    store_id: String,
    graph_hash: String,
    readiness_checker_source_sha256: String,
    root_node: String,
    root_receipt_sha256: String,
    experiment_path: String,
    report_path: String,
    series_id: String,
    agent_public_members: Vec<AgentPublicMember>,
    agent_public_set_sha256: String,
    controller_members: Vec<ControllerMember>,
    controller_set_sha256: String,
    r1_public_set_sha256: String,
    r1_controller_tree_sha256: String,
    annotation_a_sha256: String,
    annotation_b_sha256: String,
    annotation_a_receipt_sha256: String,
    annotation_b_receipt_sha256: String,
    annotation_a_path: String,
    annotation_b_path: String,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct HiddenAuthorizationEnvelope {
    schema: String,
    issuer: String,
    purpose: String,
    payload: Value,
    signature: String,
}

pub struct HiddenVerificationAuthorizationInput {
    pub readiness_store: PathBuf,
    pub authorization_path: PathBuf,
    pub root_receipt_path: PathBuf,
    pub experiment_path: PathBuf,
    pub report_path: PathBuf,
    pub annotation_a_path: PathBuf,
    pub annotation_b_path: PathBuf,
}

/// Same-process, non-serializable R1 hidden-verification capability.
pub struct HiddenVerificationAuthorization {
    input: HiddenVerificationAuthorizationInput,
    authorization_sha256: String,
    payload: HiddenAuthorizationPayload,
    verifying_key: [u8; 32],
}

struct ValidatedHidden {
    members: E04MemberSets,
    verdicts: Vec<HiddenTaskVerdict>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct HiddenVerificationReport {
    pub schema: String,
    pub authorization_envelope_sha256: String,
    pub root_receipt_sha256: String,
    pub series_id: String,
    pub experiment_path: String,
    pub report_path: String,
    pub task_count: usize,
    pub agent_public_members: Vec<AgentPublicMember>,
    pub agent_public_set_sha256: String,
    pub controller_members: Vec<ControllerMember>,
    pub controller_set_sha256: String,
    pub verified_task_ids: Vec<String>,
    pub r1_public_set_sha256: String,
    pub r1_controller_tree_sha256: String,
    pub annotation_a_sha256: String,
    pub annotation_b_sha256: String,
    pub annotation_a_receipt_sha256: String,
    pub annotation_b_receipt_sha256: String,
    pub verdicts: Vec<HiddenTaskVerdict>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct BlindAnnotation {
    pub schema: String,
    pub annotator_id: String,
    pub series_id: String,
    pub r1_public_set_sha256: String,
    pub tasks: Vec<BlindAnnotationTask>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct BlindAnnotationTask {
    pub task_id: String,
    pub family: String,
    pub outcome: ExpectedOutcome,
    pub required_obligations: Vec<String>,
    pub required_bindings: Vec<BindingPair>,
    pub ambiguous_choices: Vec<Vec<BindingPair>>,
    pub refusal_code: Option<String>,
    pub oracle_class: Option<String>,
    pub evidence: AnnotationEvidence,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct BindingPair {
    pub role: String,
    pub symbol: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct AnnotationEvidence {
    pub public_manifest_sha256: String,
    pub repository_source_sha256: String,
    pub anchors: Vec<SourceAnchor>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct SourceAnchor {
    pub symbol: String,
    pub relative_path: String,
    pub file_sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct HiddenTaskVerdict {
    pub task_id: String,
    pub family: String,
    pub outcome: ExpectedOutcome,
    pub required_obligations: Vec<String>,
    pub binding_count: usize,
    pub ambiguous_choice_count: usize,
    pub refusal_code: Option<String>,
    pub oracle_class: Option<String>,
    pub decision_sha256: String,
    pub evidence_sha256: String,
    pub status: String,
}

pub fn authorize_hidden_verification(
    input: HiddenVerificationAuthorizationInput,
) -> Result<HiddenVerificationAuthorization> {
    authorize_hidden_verification_with_key(input, production_verifying_key()?)
}

fn authorize_hidden_verification_with_key(
    input: HiddenVerificationAuthorizationInput,
    verifying_key: VerifyingKey,
) -> Result<HiddenVerificationAuthorization> {
    let (payload, authorization_sha256) =
        verify_hidden_envelope(&input.authorization_path, &verifying_key)?;
    validate_hidden_authority(&input, &payload)?;
    Ok(HiddenVerificationAuthorization {
        input,
        authorization_sha256,
        payload,
        verifying_key: verifying_key.to_bytes(),
    })
}

pub fn verify_e04_hidden(
    authorization: HiddenVerificationAuthorization,
) -> Result<HiddenVerificationReport> {
    let verifying_key = VerifyingKey::from_bytes(&authorization.verifying_key)
        .context("hidden-verification capability verifier is invalid")?;
    let (payload, authorization_sha256) =
        verify_hidden_envelope(&authorization.input.authorization_path, &verifying_key)?;
    if authorization_sha256 != authorization.authorization_sha256
        || payload.agent_public_set_sha256 != authorization.payload.agent_public_set_sha256
        || payload.controller_set_sha256 != authorization.payload.controller_set_sha256
        || payload.root_receipt_sha256 != authorization.payload.root_receipt_sha256
    {
        bail!("hidden-verification capability changed after issuance");
    }
    let validated = validate_hidden_authority(&authorization.input, &payload)?;
    let members = validated.members;
    let report = HiddenVerificationReport {
        schema: R1_HIDDEN_VERIFICATION_REPORT_SCHEMA.into(),
        authorization_envelope_sha256: authorization_sha256,
        root_receipt_sha256: payload.root_receipt_sha256,
        series_id: payload.series_id,
        experiment_path: members.canonical_root.to_string_lossy().into_owned(),
        report_path: canonical_absent_output(&authorization.input.report_path)?
            .to_string_lossy()
            .into_owned(),
        task_count: 42,
        verified_task_ids: members
            .agent_public_members
            .iter()
            .map(|member| member.task_id.clone())
            .collect(),
        agent_public_members: members.agent_public_members,
        agent_public_set_sha256: members.agent_public_set_sha256,
        controller_members: members.controller_members,
        controller_set_sha256: members.controller_set_sha256,
        r1_public_set_sha256: members.r1_public_set_sha256,
        r1_controller_tree_sha256: members.r1_controller_tree_sha256,
        annotation_a_sha256: payload.annotation_a_sha256,
        annotation_b_sha256: payload.annotation_b_sha256,
        annotation_a_receipt_sha256: payload.annotation_a_receipt_sha256,
        annotation_b_receipt_sha256: payload.annotation_b_receipt_sha256,
        verdicts: validated.verdicts,
    };
    let bytes = canonical_json(&report)?.into_bytes();
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&authorization.input.report_path)
        .context("create hidden-verification report")?;
    file.write_all(&bytes)?;
    file.sync_all()?;
    Ok(report)
}

fn verify_hidden_envelope(
    path: &Path,
    verifying_key: &VerifyingKey,
) -> Result<(HiddenAuthorizationPayload, String)> {
    let value = read_canonical_json(path, "signed hidden-verification authorization")?;
    let envelope: HiddenAuthorizationEnvelope = serde_json::from_value(value)
        .context("invalid signed hidden-verification authorization")?;
    if envelope.schema != R1_HIDDEN_AUTHORIZATION_ENVELOPE_SCHEMA
        || envelope.issuer != R1_AUTHORIZATION_ISSUER
        || envelope.purpose != R1_HIDDEN_VERIFICATION_PURPOSE
    {
        bail!("hidden-verification issuer/purpose contract mismatch");
    }
    verify_purpose_signature(
        verifying_key,
        R1_HIDDEN_VERIFICATION_PURPOSE,
        &envelope.payload,
        &envelope.signature,
    )?;
    let payload: HiddenAuthorizationPayload = serde_json::from_value(envelope.payload)
        .context("invalid hidden-verification authorization payload")?;
    Ok((payload, sha256_file(path)?))
}

fn validate_hidden_authority(
    input: &HiddenVerificationAuthorizationInput,
    payload: &HiddenAuthorizationPayload,
) -> Result<ValidatedHidden> {
    let (graph_hash, checker_hash) = pinned_readiness_contract()?;
    if payload.schema != R1_HIDDEN_AUTHORIZATION_SCHEMA
        || payload.graph_hash != graph_hash
        || payload.readiness_checker_source_sha256 != checker_hash
        || payload.root_node != R1_HIDDEN_ROOT
    {
        bail!("hidden-verification authorization targets an unpinned authority contract");
    }
    let store = canonical_directory(&input.readiness_store, "readiness store")?;
    let authorization_path = contained_content_file(
        &store,
        &input.authorization_path,
        "authorizations",
        "hidden-verification authorization",
    )?;
    if content_address(&authorization_path)? != sha256_file(&authorization_path)? {
        bail!("hidden-verification authorization content address mismatch");
    }
    let readiness = verify_pinned_readiness_root(
        &store,
        &input.root_receipt_path,
        &payload.store_id,
        &payload.graph_hash,
        &payload.root_node,
        &payload.root_receipt_sha256,
    )?;
    if readiness.store_root != store || readiness.selected_inputs.is_empty() {
        bail!("hidden-verification readiness closure is incomplete");
    }
    let report_path = canonical_absent_output(&input.report_path)?;
    if report_path.to_string_lossy() != payload.report_path {
        bail!("hidden-verification report path binding mismatch");
    }
    let members = inspect_materialized_member_sets(&input.experiment_path, &payload.series_id)?;
    if members.canonical_root.to_string_lossy() != payload.experiment_path
        || members.agent_public_members != payload.agent_public_members
        || members.agent_public_set_sha256 != payload.agent_public_set_sha256
        || members.controller_members != payload.controller_members
        || members.controller_set_sha256 != payload.controller_set_sha256
        || members.r1_public_set_sha256 != payload.r1_public_set_sha256
        || members.r1_controller_tree_sha256 != payload.r1_controller_tree_sha256
    {
        bail!("hidden-verification member-set binding mismatch");
    }
    for (selector, expected) in [
        ("r1PublicSetSha256", payload.r1_public_set_sha256.as_str()),
        (
            "r1ControllerTreeSha256",
            payload.r1_controller_tree_sha256.as_str(),
        ),
        ("r1AnnotationASha256", payload.annotation_a_sha256.as_str()),
        ("r1AnnotationBSha256", payload.annotation_b_sha256.as_str()),
    ] {
        if readiness.selected_inputs.get(selector).map(String::as_str) != Some(expected) {
            bail!("hidden-verification readiness selector mismatch: {selector}");
        }
    }
    if readiness.receipt_hashes.get("R1_BLIND_ANNOTATION_A_IMPORT")
        != Some(&payload.annotation_a_receipt_sha256)
        || readiness.receipt_hashes.get("R1_BLIND_ANNOTATION_B_IMPORT")
            != Some(&payload.annotation_b_receipt_sha256)
    {
        bail!("hidden-verification annotation receipt binding mismatch");
    }
    let verdicts = verify_annotations(input, payload, &members)?;
    Ok(ValidatedHidden { members, verdicts })
}

fn verify_annotations(
    input: &HiddenVerificationAuthorizationInput,
    payload: &HiddenAuthorizationPayload,
    members: &E04MemberSets,
) -> Result<Vec<HiddenTaskVerdict>> {
    let annotation_a = read_annotation(
        &input.annotation_a_path,
        &payload.annotation_a_path,
        &payload.annotation_a_sha256,
        R1_BLIND_ANNOTATOR_A,
    )?;
    let annotation_b = read_annotation(
        &input.annotation_b_path,
        &payload.annotation_b_path,
        &payload.annotation_b_sha256,
        R1_BLIND_ANNOTATOR_B,
    )?;
    if annotation_a.annotator_id == annotation_b.annotator_id
        || annotation_a.series_id != payload.series_id
        || annotation_b.series_id != payload.series_id
        || annotation_a.r1_public_set_sha256 != payload.r1_public_set_sha256
        || annotation_b.r1_public_set_sha256 != payload.r1_public_set_sha256
        || annotation_a.tasks != annotation_b.tasks
    {
        bail!("blind annotations disagree or do not bind the R1 public series");
    }
    let expected_task_ids = members
        .agent_public_members
        .iter()
        .map(|member| member.task_id.clone())
        .collect::<Vec<_>>();
    let annotated_task_ids = annotation_a
        .tasks
        .iter()
        .map(|task| task.task_id.clone())
        .collect::<Vec<_>>();
    if annotation_a.tasks.len() != 42
        || annotated_task_ids != expected_task_ids
        || annotated_task_ids.iter().collect::<BTreeSet<_>>().len() != 42
    {
        bail!("blind annotation task set/order differs from the exact R1 public set");
    }

    let population = crate::population::parse_and_validate(include_str!(
        "../../../benchmarks/semantic-change/editing-population-v1.json"
    ))?;
    let catalog = population
        .families
        .into_iter()
        .map(|family| (family.id, family.required_obligations))
        .collect::<BTreeMap<_, _>>();
    let public_members = members
        .agent_public_members
        .iter()
        .map(|member| (member.task_id.as_str(), member))
        .collect::<BTreeMap<_, _>>();
    let mut verdicts = Vec::with_capacity(42);
    for annotation in annotation_a.tasks {
        let obligations = catalog
            .get(&annotation.family)
            .context("blind annotation contains a family outside the frozen catalog")?;
        if &annotation.required_obligations != obligations {
            bail!("blind annotation obligations differ from the frozen family catalog");
        }
        let controller_path = members
            .canonical_root
            .join("controller")
            .join(&annotation.task_id)
            .join("manifest.json");
        let metadata = fs::symlink_metadata(&controller_path)
            .context("controller manifest disappeared during adjudication")?;
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            bail!("controller adjudication input is not a regular file");
        }
        let controller: ControllerTask = serde_json::from_slice(&fs::read(&controller_path)?)
            .context("controller adjudication manifest is invalid")?;
        let controller_bindings = normalize_bindings(&controller.required_bindings)?;
        let controller_choices = normalize_choices(&controller.ambiguous_choices)?;
        validate_canonical_annotation_decision(&annotation)?;
        let choice_count = controller_choices.len();
        let refusal_required = controller.refusal_reason.is_some();
        let controller_invariants = match controller.expected_outcome {
            ExpectedOutcome::Bound => {
                !controller_bindings.is_empty() && choice_count == 0 && !refusal_required
            }
            ExpectedOutcome::Ambiguous => choice_count >= 2 && !refusal_required,
            ExpectedOutcome::Refused => choice_count == 0 && refusal_required,
        };
        if !controller_invariants
            || controller.slot.family != annotation.family
            || controller.required_obligations != annotation.required_obligations
            || controller.expected_outcome != annotation.outcome
            || controller_bindings != annotation.required_bindings
            || controller_choices != annotation.ambiguous_choices
            || controller.refusal_reason != annotation.refusal_code
            || controller.expected_oracle_class != annotation.oracle_class
        {
            bail!("blind annotation does not match controller truth/invariants");
        }
        let member = public_members
            .get(annotation.task_id.as_str())
            .context("blind annotation task is absent from the public member set")?;
        validate_annotation_evidence(&members.canonical_root, member, &annotation)?;
        let decision = serde_json::json!({
            "family": annotation.family,
            "outcome": annotation.outcome,
            "requiredObligations": annotation.required_obligations,
            "requiredBindings": annotation.required_bindings,
            "ambiguousChoices": annotation.ambiguous_choices,
            "refusalCode": annotation.refusal_code,
            "oracleClass": annotation.oracle_class,
        });
        let decision_sha256 = sha256_bytes(canonical_json(&decision)?.as_bytes());
        let evidence_sha256 = sha256_bytes(canonical_json(&annotation.evidence)?.as_bytes());
        verdicts.push(HiddenTaskVerdict {
            task_id: annotation.task_id,
            family: annotation.family,
            outcome: annotation.outcome,
            required_obligations: annotation.required_obligations,
            binding_count: controller_bindings.len(),
            ambiguous_choice_count: choice_count,
            refusal_code: annotation.refusal_code,
            oracle_class: annotation.oracle_class,
            decision_sha256,
            evidence_sha256,
            status: "VERIFIED".into(),
        });
    }
    Ok(verdicts)
}

fn normalize_bindings(bindings: &[String]) -> Result<Vec<BindingPair>> {
    let mut normalized = bindings
        .iter()
        .map(|binding| {
            let (role, symbol) = binding
                .split_once('=')
                .context("controller binding must have exact ROLE=SYMBOL form")?;
            if role.is_empty()
                || symbol.is_empty()
                || role.contains(char::is_whitespace)
                || symbol.contains(char::is_whitespace)
                || symbol.contains('=')
            {
                bail!("controller binding contains an invalid role or symbol");
            }
            Ok(BindingPair {
                role: role.into(),
                symbol: symbol.into(),
            })
        })
        .collect::<Result<Vec<_>>>()?;
    normalized.sort();
    if normalized.windows(2).any(|pair| pair[0] == pair[1]) {
        bail!("controller binding set contains duplicates");
    }
    Ok(normalized)
}

fn normalize_choices(choices: &[Vec<String>]) -> Result<Vec<Vec<BindingPair>>> {
    let mut normalized = choices
        .iter()
        .map(|choice| {
            if choice.is_empty() {
                bail!("ambiguous binding choice cannot be empty");
            }
            normalize_bindings(choice)
        })
        .collect::<Result<Vec<_>>>()?;
    normalized.sort_by_key(|choice| canonical_json(choice).unwrap_or_default());
    if normalized.windows(2).any(|pair| pair[0] == pair[1]) {
        bail!("ambiguous binding choices contain duplicates");
    }
    Ok(normalized)
}

fn validate_canonical_annotation_decision(annotation: &BlindAnnotationTask) -> Result<()> {
    let mut bindings = annotation.required_bindings.clone();
    bindings.sort();
    let mut choices = annotation.ambiguous_choices.clone();
    for choice in &mut choices {
        choice.sort();
    }
    choices.sort_by_key(|choice| canonical_json(choice).unwrap_or_default());
    if bindings != annotation.required_bindings
        || choices != annotation.ambiguous_choices
        || bindings.windows(2).any(|pair| pair[0] == pair[1])
        || choices.windows(2).any(|pair| pair[0] == pair[1])
        || annotation
            .ambiguous_choices
            .iter()
            .any(|choice| choice.is_empty() || choice.windows(2).any(|pair| pair[0] == pair[1]))
    {
        bail!("blind annotation decision fields are not canonical sets");
    }
    Ok(())
}

fn validate_annotation_evidence(
    experiment_root: &Path,
    member: &AgentPublicMember,
    annotation: &BlindAnnotationTask,
) -> Result<()> {
    if annotation.evidence.public_manifest_sha256 != member.public_manifest_sha256
        || annotation.evidence.repository_source_sha256 != member.repository_source_sha256
    {
        bail!("blind annotation evidence does not bind the public task snapshot");
    }
    let expected_symbols = annotation
        .required_bindings
        .iter()
        .chain(annotation.ambiguous_choices.iter().flatten())
        .map(|binding| binding.symbol.clone())
        .collect::<BTreeSet<_>>();
    let mut anchors = annotation.evidence.anchors.clone();
    anchors.sort();
    if anchors != annotation.evidence.anchors
        || anchors.windows(2).any(|pair| pair[0] == pair[1])
        || anchors
            .iter()
            .map(|anchor| anchor.symbol.clone())
            .collect::<BTreeSet<_>>()
            != expected_symbols
        || anchors.len() != expected_symbols.len()
    {
        bail!("blind annotation evidence anchors are not an exact canonical symbol set");
    }
    let repository = experiment_root
        .join("agent")
        .join(&annotation.task_id)
        .join("repository");
    let canonical_repository =
        fs::canonicalize(&repository).context("blind annotation repository snapshot is missing")?;
    for anchor in &anchors {
        validate_source_anchor(&repository, &canonical_repository, anchor)?;
    }
    Ok(())
}

fn validate_source_anchor(
    repository: &Path,
    canonical_repository: &Path,
    anchor: &SourceAnchor,
) -> Result<()> {
    let relative = Path::new(&anchor.relative_path);
    if relative.is_absolute()
        || relative
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        bail!("blind annotation source anchor path is not a safe relative path");
    }
    let mut current = repository.to_path_buf();
    for component in relative.components() {
        let Component::Normal(component) = component else {
            unreachable!()
        };
        current.push(component);
        let metadata = fs::symlink_metadata(&current)
            .context("blind annotation source anchor path is missing")?;
        if metadata.file_type().is_symlink() {
            bail!("blind annotation source anchor traverses a symlink");
        }
    }
    let metadata = fs::symlink_metadata(&current)?;
    if !metadata.is_file() {
        bail!("blind annotation source anchor is not a regular file");
    }
    let canonical = fs::canonicalize(&current)?;
    if !canonical.starts_with(canonical_repository) || sha256_file(&current)? != anchor.file_sha256
    {
        bail!("blind annotation source anchor digest/containment mismatch");
    }
    let source = fs::read_to_string(&current)
        .context("blind annotation source anchor is not UTF-8 source")?;
    let terminal = symbol_terminal(&anchor.symbol)?;
    if !contains_identifier(&source, terminal) {
        bail!("blind annotation source anchor does not contain its symbol identifier");
    }
    Ok(())
}

fn symbol_terminal(symbol: &str) -> Result<&str> {
    let callable = symbol.split_once('(').map_or(symbol, |(head, _)| head);
    let terminal = callable
        .rsplit(['/', '.', '$', '#', ':'])
        .next()
        .unwrap_or_default()
        .trim_matches('`');
    if terminal.is_empty()
        || !terminal
            .chars()
            .all(|character| character == '_' || character.is_alphanumeric())
    {
        bail!("blind annotation symbol has no source-verifiable terminal identifier");
    }
    Ok(terminal)
}

fn contains_identifier(source: &str, identifier: &str) -> bool {
    source.match_indices(identifier).any(|(start, _)| {
        let before = source[..start].chars().next_back();
        let after = source[start + identifier.len()..].chars().next();
        !before.is_some_and(|character| character == '_' || character.is_alphanumeric())
            && !after.is_some_and(|character| character == '_' || character.is_alphanumeric())
    })
}

fn read_annotation(
    path: &Path,
    expected_path: &str,
    expected_sha256: &str,
    expected_annotator: &str,
) -> Result<BlindAnnotation> {
    let metadata = fs::symlink_metadata(path).context("blind annotation input is missing")?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        bail!("blind annotation must be a regular non-symlink file");
    }
    let canonical_path = fs::canonicalize(path)?;
    if canonical_path.to_string_lossy() != expected_path || sha256_file(path)? != expected_sha256 {
        bail!("blind annotation path/digest binding mismatch");
    }
    let value = read_canonical_json(path, "blind annotation")?;
    let annotation: BlindAnnotation =
        serde_json::from_value(value).context("invalid blind annotation schema")?;
    if annotation.schema != R1_BLIND_ANNOTATION_SCHEMA
        || annotation.annotator_id != expected_annotator
    {
        bail!("blind annotation identity is invalid");
    }
    Ok(annotation)
}

fn sha256_file(path: &Path) -> Result<String> {
    use sha2::{Digest, Sha256};
    Ok(hex::encode(Sha256::digest(fs::read(path)?)))
}

fn sha256_bytes(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    hex::encode(Sha256::digest(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    use ed25519_dalek::{Signer, SigningKey};
    use serde_json::json;
    use sha2::{Digest, Sha256};

    use crate::e04::tests::materialization_result_fixture;
    use crate::e04_authorization::canonical_json_bytes;

    const STORE_SCHEMA: &str = "semantic-editing-e04-readiness-store/0.1";
    const GRAPH_SCHEMA: &str = "semantic-editing-e04-readiness-graph/0.1";
    const POINTER_SCHEMA: &str = "semantic-editing-e04-readiness-pointer/0.1";
    const RECEIPT_SCHEMA: &str = "semantic-editing-e04-readiness-receipt/0.1";
    const CHECKER_VERSION: &str = "e04-readiness-phase1/0.1";

    #[derive(Clone)]
    struct NodeSpec {
        checker: String,
        dependencies: Vec<String>,
        selectors: Vec<String>,
    }

    struct Fixture {
        _temporary: tempfile::TempDir,
        store: PathBuf,
        authorization: PathBuf,
        root_receipt: PathBuf,
        experiment: PathBuf,
        report: PathBuf,
        payload: HiddenAuthorizationPayload,
        signing_key: [u8; 32],
        annotation_a: PathBuf,
        annotation_b: PathBuf,
    }

    impl Fixture {
        fn input(&self) -> HiddenVerificationAuthorizationInput {
            HiddenVerificationAuthorizationInput {
                readiness_store: self.store.clone(),
                authorization_path: self.authorization.clone(),
                root_receipt_path: self.root_receipt.clone(),
                experiment_path: self.experiment.clone(),
                report_path: self.report.clone(),
                annotation_a_path: self.annotation_a.clone(),
                annotation_b_path: self.annotation_b.clone(),
            }
        }
    }

    fn hash(bytes: &[u8]) -> String {
        hex::encode(Sha256::digest(bytes))
    }

    fn write_canonical(path: &Path, value: &Value) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, canonical_json_bytes(value)).unwrap();
    }

    fn write_object(store: &Path, value: &Value) -> (String, PathBuf) {
        let bytes = canonical_json_bytes(value);
        let identity = hash(&bytes);
        let path = store.join("objects").join(format!("{identity}.json"));
        fs::write(&path, bytes).unwrap();
        (identity, path)
    }

    fn graph_specs(graph: &Value) -> BTreeMap<String, NodeSpec> {
        graph["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|node| {
                let id = node["id"].as_str().unwrap().to_owned();
                let spec = NodeSpec {
                    checker: node["checker"].as_str().unwrap().to_owned(),
                    dependencies: node["dependencies"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|value| value.as_str().unwrap().to_owned())
                        .collect(),
                    selectors: node["inputSelectors"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|value| value.as_str().unwrap().to_owned())
                        .collect(),
                };
                (id, spec)
            })
            .collect()
    }

    #[allow(clippy::too_many_arguments)]
    fn build_chain(
        store: &Path,
        store_id: &str,
        graph_hash: &str,
        checker_hash: &str,
        specs: &BTreeMap<String, NodeSpec>,
        selected_values: &BTreeMap<String, String>,
        node: &str,
        written: &mut BTreeMap<String, (String, PathBuf)>,
        created: &mut u64,
    ) -> (String, PathBuf) {
        if let Some(existing) = written.get(node) {
            return existing.clone();
        }
        let spec = specs.get(node).unwrap();
        let dependencies = spec
            .dependencies
            .iter()
            .map(|dependency| {
                let (receipt, _) = build_chain(
                    store,
                    store_id,
                    graph_hash,
                    checker_hash,
                    specs,
                    selected_values,
                    dependency,
                    written,
                    created,
                );
                (dependency.clone(), Value::String(receipt))
            })
            .collect::<serde_json::Map<_, _>>();
        let selected = spec
            .selectors
            .iter()
            .map(|selector| {
                (
                    selector.clone(),
                    Value::String(
                        selected_values
                            .get(selector)
                            .cloned()
                            .unwrap_or_else(|| hash(selector.as_bytes())),
                    ),
                )
            })
            .collect::<serde_json::Map<_, _>>();
        let node_key = hash(&canonical_json_bytes(&json!({
            "storeId":store_id,"graphHash":graph_hash,"checkerVersion":CHECKER_VERSION,
            "checker":spec.checker.as_str(),"checkerSourceSha256":checker_hash,"node":node,
            "inputs":&selected,"dependencies":&dependencies
        })));
        *created += 1;
        let receipt = json!({
            "schema":RECEIPT_SCHEMA,"storeId":store_id,"graphHash":graph_hash,
            "checkerVersion":CHECKER_VERSION,"node":node,"nodeKey":node_key,
            "status":"READY","selectedInputs":selected,"dependencies":dependencies,
            "evidence":{},"error":null,"createdUnixNs":*created
        });
        let result = write_object(store, &receipt);
        write_canonical(
            &store.join("current").join(format!("{node}.json")),
            &json!({"schema":POINTER_SCHEMA,"storeId":store_id,"graphHash":graph_hash,
                "node":node,"receiptHash":result.0}),
        );
        written.insert(node.to_owned(), result.clone());
        result
    }

    fn install_envelope(
        fixture: &mut Fixture,
        payload: HiddenAuthorizationPayload,
        signing_key: [u8; 32],
        purpose: &str,
    ) {
        let payload_value = serde_json::to_value(&payload).unwrap();
        let mut message = Vec::from(purpose.as_bytes());
        message.push(0);
        message.extend(canonical_json_bytes(&payload_value));
        let signature = SigningKey::from_bytes(&signing_key).sign(&message);
        let envelope = json!({
            "schema":R1_HIDDEN_AUTHORIZATION_ENVELOPE_SCHEMA,
            "issuer":R1_AUTHORIZATION_ISSUER,"purpose":R1_HIDDEN_VERIFICATION_PURPOSE,
            "payload":payload_value,"signature":hex::encode(signature.to_bytes())
        });
        let bytes = canonical_json_bytes(&envelope);
        let identity = hash(&bytes);
        let path = fixture
            .store
            .join("authorizations")
            .join(format!("{identity}.json"));
        fs::write(&path, bytes).unwrap();
        fixture.authorization = path;
        fixture.payload = payload;
    }

    fn fixture() -> Fixture {
        let (temporary, binding) = materialization_result_fixture();
        let experiment = binding.output_path.clone();
        let members = inspect_materialized_member_sets(&experiment, &binding.series_id).unwrap();
        let store = temporary.path().join("hidden-readiness");
        for directory in ["objects", "current", "graphs", "authorizations"] {
            fs::create_dir_all(store.join(directory)).unwrap();
        }
        let store_id = "1".repeat(64);
        write_canonical(
            &store.join("STORE.json"),
            &json!({"schema":STORE_SCHEMA,"storeId":store_id}),
        );
        let graph: Value = serde_json::from_str(include_str!(
            "../../../benchmarks/semantic-change/e04-readiness-graph.json"
        ))
        .unwrap();
        assert_eq!(graph["schema"], GRAPH_SCHEMA);
        let graph_bytes = canonical_json_bytes(&graph);
        let graph_hash = hash(&graph_bytes);
        fs::write(
            store.join("graphs").join(format!("{graph_hash}.json")),
            graph_bytes,
        )
        .unwrap();
        let checker_hash = hash(include_bytes!("../../../scripts/e04_readiness.py"));
        let annotation_tasks = members
            .agent_public_members
            .iter()
            .map(|member| {
                let controller: ControllerTask = serde_json::from_slice(
                    &fs::read(
                        experiment
                            .join("controller")
                            .join(&member.task_id)
                            .join("manifest.json"),
                    )
                    .unwrap(),
                )
                .unwrap();
                let required_bindings = normalize_bindings(&controller.required_bindings).unwrap();
                let ambiguous_choices = normalize_choices(&controller.ambiguous_choices).unwrap();
                let source = experiment
                    .join("agent")
                    .join(&member.task_id)
                    .join("repository/source.kt");
                let symbols = required_bindings
                    .iter()
                    .chain(ambiguous_choices.iter().flatten())
                    .map(|binding| binding.symbol.clone())
                    .collect::<BTreeSet<_>>();
                BlindAnnotationTask {
                    task_id: member.task_id.clone(),
                    family: controller.slot.family,
                    outcome: controller.expected_outcome,
                    required_obligations: controller.required_obligations,
                    required_bindings,
                    ambiguous_choices,
                    refusal_code: controller.refusal_reason,
                    oracle_class: controller.expected_oracle_class,
                    evidence: AnnotationEvidence {
                        public_manifest_sha256: member.public_manifest_sha256.clone(),
                        repository_source_sha256: member.repository_source_sha256.clone(),
                        anchors: symbols
                            .into_iter()
                            .map(|symbol| SourceAnchor {
                                symbol,
                                relative_path: "source.kt".into(),
                                file_sha256: sha256_file(&source).unwrap(),
                            })
                            .collect(),
                    },
                }
            })
            .collect::<Vec<_>>();
        let annotation_a_value = serde_json::to_value(BlindAnnotation {
            schema: R1_BLIND_ANNOTATION_SCHEMA.into(),
            annotator_id: R1_BLIND_ANNOTATOR_A.into(),
            series_id: binding.series_id.clone(),
            r1_public_set_sha256: members.r1_public_set_sha256.clone(),
            tasks: annotation_tasks.clone(),
        })
        .unwrap();
        let annotation_b_value = serde_json::to_value(BlindAnnotation {
            schema: R1_BLIND_ANNOTATION_SCHEMA.into(),
            annotator_id: R1_BLIND_ANNOTATOR_B.into(),
            series_id: binding.series_id.clone(),
            r1_public_set_sha256: members.r1_public_set_sha256.clone(),
            tasks: annotation_tasks,
        })
        .unwrap();
        let annotation_a = temporary.path().join("annotation-a.json");
        let annotation_b = temporary.path().join("annotation-b.json");
        write_canonical(&annotation_a, &annotation_a_value);
        write_canonical(&annotation_b, &annotation_b_value);
        let annotation_a_sha256 = sha256_file(&annotation_a).unwrap();
        let annotation_b_sha256 = sha256_file(&annotation_b).unwrap();
        let selected_values = BTreeMap::from([
            (
                "r1PublicSetSha256".into(),
                members.r1_public_set_sha256.clone(),
            ),
            (
                "r1ControllerTreeSha256".into(),
                members.r1_controller_tree_sha256.clone(),
            ),
            ("r1AnnotationASha256".into(), annotation_a_sha256.clone()),
            ("r1AnnotationBSha256".into(), annotation_b_sha256.clone()),
        ]);
        let specs = graph_specs(&graph);
        let mut written = BTreeMap::new();
        let mut created = 0;
        let (root_hash, root_receipt) = build_chain(
            &store,
            &store_id,
            &graph_hash,
            &checker_hash,
            &specs,
            &selected_values,
            R1_HIDDEN_ROOT,
            &mut written,
            &mut created,
        );
        let annotation_a_receipt_sha256 = written
            .get("R1_BLIND_ANNOTATION_A_IMPORT")
            .unwrap()
            .0
            .clone();
        let annotation_b_receipt_sha256 = written
            .get("R1_BLIND_ANNOTATION_B_IMPORT")
            .unwrap()
            .0
            .clone();
        let report = temporary.path().join("hidden-report.json");
        let payload = HiddenAuthorizationPayload {
            schema: R1_HIDDEN_AUTHORIZATION_SCHEMA.into(),
            store_id,
            graph_hash,
            readiness_checker_source_sha256: checker_hash,
            root_node: R1_HIDDEN_ROOT.into(),
            root_receipt_sha256: root_hash,
            experiment_path: experiment.to_string_lossy().into_owned(),
            report_path: fs::canonicalize(temporary.path())
                .unwrap()
                .join("hidden-report.json")
                .to_string_lossy()
                .into_owned(),
            series_id: binding.series_id,
            agent_public_members: members.agent_public_members,
            agent_public_set_sha256: members.agent_public_set_sha256,
            controller_members: members.controller_members,
            controller_set_sha256: members.controller_set_sha256,
            r1_public_set_sha256: members.r1_public_set_sha256,
            r1_controller_tree_sha256: members.r1_controller_tree_sha256,
            annotation_a_sha256,
            annotation_b_sha256,
            annotation_a_receipt_sha256,
            annotation_b_receipt_sha256,
            annotation_a_path: fs::canonicalize(&annotation_a)
                .unwrap()
                .to_string_lossy()
                .into_owned(),
            annotation_b_path: fs::canonicalize(&annotation_b)
                .unwrap()
                .to_string_lossy()
                .into_owned(),
        };
        let signing_key = [61; 32];
        let mut result = Fixture {
            _temporary: temporary,
            store,
            authorization: PathBuf::new(),
            root_receipt,
            experiment,
            report,
            payload: payload.clone(),
            signing_key,
            annotation_a,
            annotation_b,
        };
        install_envelope(
            &mut result,
            payload,
            signing_key,
            R1_HIDDEN_VERIFICATION_PURPOSE,
        );
        result
    }

    fn authorize(fixture: &Fixture) -> Result<HiddenVerificationAuthorization> {
        authorize_hidden_verification_with_key(
            fixture.input(),
            SigningKey::from_bytes(&fixture.signing_key).verifying_key(),
        )
    }

    #[test]
    fn r1_hidden_verification_accepts_exact_signed_closure_and_writes_canonical_report() {
        let fixture = fixture();
        let report = verify_e04_hidden(authorize(&fixture).unwrap()).unwrap();
        assert_eq!(report.task_count, 42);
        assert_eq!(report.verified_task_ids.len(), 42);
        assert_eq!(
            fs::read_to_string(&fixture.report).unwrap(),
            canonical_json(&report).unwrap()
        );
    }

    #[test]
    fn r1_hidden_verification_refuses_unsigned_self_signed_and_wrong_purpose() {
        let mut unsigned = fixture();
        let mut envelope = read_canonical_json(&unsigned.authorization, "test envelope").unwrap();
        envelope["signature"] = Value::String("0".repeat(128));
        let bytes = canonical_json_bytes(&envelope);
        let identity = hash(&bytes);
        unsigned.authorization = unsigned
            .store
            .join("authorizations")
            .join(format!("{identity}.json"));
        fs::write(&unsigned.authorization, bytes).unwrap();
        assert!(authorize(&unsigned).is_err());

        let mut self_signed = fixture();
        let payload = self_signed.payload.clone();
        install_envelope(
            &mut self_signed,
            payload,
            [62; 32],
            R1_HIDDEN_VERIFICATION_PURPOSE,
        );
        assert!(authorize(&self_signed).is_err());

        let mut wrong_purpose = fixture();
        let payload = wrong_purpose.payload.clone();
        let key = wrong_purpose.signing_key;
        install_envelope(
            &mut wrong_purpose,
            payload,
            key,
            "codeclew/e04/r1-materialization/0.1",
        );
        assert!(authorize(&wrong_purpose).is_err());
    }

    #[test]
    fn r1_hidden_verification_refuses_wrong_root_set_and_report_path() {
        for kind in ["root", "set", "path", "annotation-receipt"] {
            let mut fixture = fixture();
            let mut payload = fixture.payload.clone();
            match kind {
                "root" => payload.root_node = "R1_MATERIALIZE_START_READY".into(),
                "set" => payload.agent_public_set_sha256 = "9".repeat(64),
                "path" => payload.report_path.push_str("-other"),
                "annotation-receipt" => payload.annotation_a_receipt_sha256 = "8".repeat(64),
                _ => unreachable!(),
            }
            let key = fixture.signing_key;
            install_envelope(&mut fixture, payload, key, R1_HIDDEN_VERIFICATION_PURPOSE);
            assert!(authorize(&fixture).is_err(), "accepted {kind}");
        }
    }

    #[test]
    fn r1_hidden_verification_refuses_partial_symlink_and_generic_bypass() {
        let partial = fixture();
        fs::remove_dir_all(partial.experiment.join("controller/e04-result-41")).unwrap();
        assert!(authorize(&partial).is_err());

        let bypass = fixture();
        let agent = bypass.experiment.join("agent/e04-result-00");
        let controller = bypass.experiment.join("controller/e04-result-00");
        let error = crate::verify_hidden_package(&agent, &controller).unwrap_err();
        assert!(error.to_string().contains("signed R1"));

        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;
            let linked = fixture();
            let manifest = linked
                .experiment
                .join("controller/e04-result-00/manifest.json");
            let external = linked._temporary.path().join("external-hidden.json");
            fs::rename(&manifest, &external).unwrap();
            symlink(&external, &manifest).unwrap();
            assert!(authorize(&linked).is_err());
        }
    }

    #[test]
    fn r1_hidden_verification_adjudication_refuses_disagreement_and_leakage() {
        let disagreement = fixture();
        let members = inspect_materialized_member_sets(
            &disagreement.experiment,
            &disagreement.payload.series_id,
        )
        .unwrap();
        let mut annotation = read_canonical_json(&disagreement.annotation_b, "annotation").unwrap();
        annotation["tasks"][0]["outcome"] = Value::String("REFUSED".into());
        write_canonical(&disagreement.annotation_b, &annotation);
        let mut payload = disagreement.payload.clone();
        payload.annotation_b_sha256 = sha256_file(&disagreement.annotation_b).unwrap();
        assert!(verify_annotations(&disagreement.input(), &payload, &members).is_err());

        let leakage = fixture();
        let members =
            inspect_materialized_member_sets(&leakage.experiment, &leakage.payload.series_id)
                .unwrap();
        let mut annotation = read_canonical_json(&leakage.annotation_a, "annotation").unwrap();
        annotation["tasks"][0]["arm"] = Value::String("default".into());
        write_canonical(&leakage.annotation_a, &annotation);
        let mut payload = leakage.payload.clone();
        payload.annotation_a_sha256 = sha256_file(&leakage.annotation_a).unwrap();
        assert!(verify_annotations(&leakage.input(), &payload, &members).is_err());

        // This is the executable false-READY counterexample for the former
        // count-only annotation contract: all counts/outcomes stay unchanged,
        // but one hidden truth binding is relabelled.
        let symbol_mutation = fixture();
        let members = inspect_materialized_member_sets(
            &symbol_mutation.experiment,
            &symbol_mutation.payload.series_id,
        )
        .unwrap();
        let controller_path = symbol_mutation
            .experiment
            .join("controller/e04-result-00/manifest.json");
        let mut controller: Value =
            serde_json::from_slice(&fs::read(&controller_path).unwrap()).unwrap();
        controller["requiredBindings"][0] = Value::String("DECLARATION=p.b".into());
        write_canonical(&controller_path, &controller);
        assert!(
            verify_annotations(&symbol_mutation.input(), &symbol_mutation.payload, &members)
                .is_err(),
            "accepted a same-cardinality hidden binding-symbol mutation"
        );
    }
}
