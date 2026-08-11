//! Fail-closed authorization for decision-bearing E04 materialization.
//!
//! The serialized authorization is evidence, not a capability by itself.  A
//! capability is issued only after the current content-addressed readiness
//! chain, decision freeze, output path, and caller-held seeds have all been
//! revalidated.  The capability deliberately has no serde, clone, or public
//! constructor surface.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Component, Path, PathBuf};

use anyhow::{Context, Result, bail};
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::GENERATOR_VERSION;
use crate::e04::{
    FROZEN_BINDER_TREE_SHA256, FROZEN_POPULATION_SHA256, FROZEN_PRODUCT_REVISION,
    MaterializeOptions,
};

pub const R1_AUTHORIZATION_SCHEMA: &str =
    "semantic-editing-e04-r1-materialization-authorization/0.1";
pub const R1_ROOT_NODE: &str = "R1_MATERIALIZE_START_READY";
pub const MATERIALIZER_CONTRACT_VERSION: &str = "semantic-corpus-e04-materializer/0.1";
pub const R1_AUTHORIZATION_ENVELOPE_SCHEMA: &str =
    "semantic-editing-e04-r1-materialization-authorization-envelope/0.1";
pub const R1_AUTHORIZATION_PURPOSE: &str = "codeclew/e04/r1-materialization/0.1";
pub const R1_AUTHORIZATION_ISSUER: &str = "codeclew-e04-production-2026-08";
const R1_AUTHORIZATION_VERIFYING_KEY_HEX: &str =
    "8bf9107a5274f66b454a74b0d6b64c7467145c3eb8a5c902ef108557345f4981";
const READINESS_RECEIPT_SCHEMA: &str = "semantic-editing-e04-readiness-receipt/0.1";
const READINESS_POINTER_SCHEMA: &str = "semantic-editing-e04-readiness-pointer/0.1";
const READINESS_STORE_SCHEMA: &str = "semantic-editing-e04-readiness-store/0.1";
const READINESS_GRAPH_SCHEMA: &str = "semantic-editing-e04-readiness-graph/0.1";
const HEX_SHA256_LEN: usize = 64;
const MAX_SECRET_BYTES: usize = 4096;
const READINESS_CHECKER_VERSION: &str = "e04-readiness-phase1/0.1";
const PINNED_READINESS_GRAPH: &[u8] =
    include_bytes!("../../../benchmarks/semantic-change/e04-readiness-graph.json");
const PINNED_READINESS_CHECKER: &[u8] = include_bytes!("../../../scripts/e04_readiness.py");

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct MaterializerIdentity {
    pub contract_version: String,
    pub generator_version: String,
    pub frozen_product_revision: String,
    pub frozen_population_sha256: String,
    pub frozen_binder_tree_sha256: String,
    pub population_spec_sha256: String,
    pub generator_source_sha256: String,
    pub e04_source_sha256: String,
    pub population_source_sha256: String,
    pub authorization_source_sha256: String,
    pub readiness_graph_sha256: String,
    pub readiness_checker_source_sha256: String,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct SerializedAuthorization {
    schema: String,
    store_id: String,
    graph_hash: String,
    root_node: String,
    root_receipt_sha256: String,
    decision_freeze_sha256: String,
    output_path: String,
    agent_seed_sha256: String,
    controller_seed_sha256: String,
    series_nonce_sha256: String,
    series_id: String,
    materializer: MaterializerIdentity,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct SignedAuthorizationEnvelope {
    schema: String,
    issuer: String,
    purpose: String,
    payload: SerializedAuthorization,
    signature: String,
}

#[derive(Clone)]
struct ReadinessNodeSpec {
    checker: String,
    dependencies: Vec<String>,
    input_selectors: Vec<String>,
}

/// Secret-bearing authorization request.  It intentionally implements neither
/// `Debug` nor serde so raw R1 seed material cannot enter diagnostics.
pub struct MaterializationAuthorizationInput {
    pub readiness_store: PathBuf,
    pub authorization_path: PathBuf,
    pub root_receipt_path: PathBuf,
    pub output_path: PathBuf,
    pub agent_seed: String,
    pub controller_seed: String,
    pub series_nonce: String,
}

/// Same-process capability required by [`crate::e04::materialize`].
///
/// Private fields, lack of serde/Clone, and consumption by materialization
/// prevent a serialized summary from authorizing corpus writes.
pub struct MaterializationAuthorization {
    store_root: PathBuf,
    store_id: String,
    authorization_path: PathBuf,
    root_receipt_path: PathBuf,
    authorization_sha256: String,
    graph_hash: String,
    root_receipt_sha256: String,
    decision_freeze_sha256: String,
    output_path: PathBuf,
    agent_seed: String,
    controller_seed: String,
    series_nonce: String,
    series_id: String,
    verifying_key: [u8; 32],
}

pub fn materializer_identity() -> MaterializerIdentity {
    MaterializerIdentity {
        contract_version: MATERIALIZER_CONTRACT_VERSION.into(),
        generator_version: GENERATOR_VERSION.into(),
        frozen_product_revision: FROZEN_PRODUCT_REVISION.into(),
        frozen_population_sha256: FROZEN_POPULATION_SHA256.into(),
        frozen_binder_tree_sha256: FROZEN_BINDER_TREE_SHA256.into(),
        population_spec_sha256: sha256(include_bytes!(
            "../../../benchmarks/semantic-change/editing-population-v1.json"
        )),
        generator_source_sha256: sha256(include_bytes!("lib.rs")),
        e04_source_sha256: sha256(include_bytes!("e04.rs")),
        population_source_sha256: sha256(include_bytes!("population.rs")),
        authorization_source_sha256: sha256(include_bytes!("e04_authorization.rs")),
        readiness_graph_sha256: pinned_readiness_graph()
            .expect("embedded readiness graph is valid")
            .1,
        readiness_checker_source_sha256: sha256(PINNED_READINESS_CHECKER),
    }
}

pub fn materializer_contract_sha256() -> String {
    sha256(&canonical_bytes(
        &serde_json::to_value(materializer_identity()).expect("materializer identity serializes"),
    ))
}

pub fn authorize_materialization(
    input: MaterializationAuthorizationInput,
) -> Result<MaterializationAuthorization> {
    let verifying_key = production_verifying_key()?;
    authorize_materialization_with_verifier(input, verifying_key)
}

fn authorize_materialization_with_verifier(
    input: MaterializationAuthorizationInput,
    verifying_key: VerifyingKey,
) -> Result<MaterializationAuthorization> {
    validate_secret("agent seed", &input.agent_seed)?;
    validate_secret("controller seed", &input.controller_seed)?;
    validate_secret("series nonce", &input.series_nonce)?;

    // The issuer signature is the trust root. Verify it before consulting any
    // caller-selected readiness store or accepting its content-addressed data.
    let (authorization, authorization_sha256) =
        verify_authorization_envelope(&input.authorization_path, &verifying_key)?;

    let store_root = canonical_directory(&input.readiness_store, "readiness store")?;
    for directory in ["objects", "current", "graphs", "authorizations"] {
        canonical_directory(&store_root.join(directory), directory)?;
    }
    let store: Value = read_canonical_json(&store_root.join("STORE.json"), "readiness store")?;
    require_exact_keys(&store, &["schema", "storeId"], "readiness store")?;
    if string(&store, "schema")? != READINESS_STORE_SCHEMA {
        bail!("unsupported readiness store schema");
    }
    let store_id = checked_sha(string(&store, "storeId")?, "readiness store ID")?;

    let authorization_path = contained_content_file(
        &store_root,
        &input.authorization_path,
        "authorizations",
        "materialization authorization",
    )?;
    if content_address(&authorization_path)? != authorization_sha256 {
        bail!("materialization authorization content address mismatch");
    }
    if authorization.schema != R1_AUTHORIZATION_SCHEMA
        || authorization.store_id != store_id
        || authorization.root_node != R1_ROOT_NODE
    {
        bail!("materialization authorization store/root contract mismatch");
    }
    for (label, value) in [
        ("graph hash", authorization.graph_hash.as_str()),
        (
            "root receipt hash",
            authorization.root_receipt_sha256.as_str(),
        ),
        (
            "decision freeze hash",
            authorization.decision_freeze_sha256.as_str(),
        ),
        ("agent seed hash", authorization.agent_seed_sha256.as_str()),
        (
            "controller seed hash",
            authorization.controller_seed_sha256.as_str(),
        ),
        (
            "series nonce hash",
            authorization.series_nonce_sha256.as_str(),
        ),
        ("series ID", authorization.series_id.as_str()),
    ] {
        checked_sha(value, label)?;
    }
    if authorization.materializer != materializer_identity() {
        bail!("materialization authorization targets a stale materializer identity");
    }
    let agent_seed_sha256 = sha256(input.agent_seed.as_bytes());
    let controller_seed_sha256 = sha256(input.controller_seed.as_bytes());
    let series_nonce_sha256 = sha256(input.series_nonce.as_bytes());
    let series_id = derive_series_id(
        input.agent_seed.as_bytes(),
        input.controller_seed.as_bytes(),
        input.series_nonce.as_bytes(),
    );
    if authorization.agent_seed_sha256 != agent_seed_sha256
        || authorization.controller_seed_sha256 != controller_seed_sha256
        || authorization.series_nonce_sha256 != series_nonce_sha256
        || authorization.series_id != series_id
    {
        bail!("materialization authorization seed/series binding mismatch");
    }

    let output_path = canonical_absent_output(&input.output_path)?;
    if authorization.output_path != output_path.to_string_lossy() {
        bail!("materialization authorization output path mismatch");
    }

    let (pinned_graph, pinned_graph_hash) = pinned_readiness_graph()?;
    if authorization.graph_hash != pinned_graph_hash {
        bail!("materialization authorization targets an unpinned readiness graph");
    }
    let graph_path = contained_content_file(
        &store_root,
        &store_root
            .join("graphs")
            .join(format!("{}.json", authorization.graph_hash)),
        "graphs",
        "readiness graph",
    )?;
    let graph = read_canonical_json(&graph_path, "readiness graph")?;
    if content_address(&graph_path)? != authorization.graph_hash
        || string(&graph, "schema")? != READINESS_GRAPH_SCHEMA
        || graph != pinned_graph
    {
        bail!("readiness graph hash/schema mismatch");
    }
    let nodes = graph_node_specs(&graph)?;
    if !nodes.contains_key(R1_ROOT_NODE) || !graph_roots(&graph)?.contains(R1_ROOT_NODE) {
        bail!("readiness graph does not declare the R1 materialization root");
    }

    let root_receipt_path = contained_content_file(
        &store_root,
        &input.root_receipt_path,
        "objects",
        "R1 root receipt",
    )?;
    if content_address(&root_receipt_path)? != authorization.root_receipt_sha256 {
        bail!("R1 root receipt content address mismatch");
    }
    let mut visited = BTreeMap::new();
    let mut selected_inputs = BTreeMap::new();
    validate_ready_receipt_chain(
        &store_root,
        &authorization.graph_hash,
        &store_id,
        &nodes,
        R1_ROOT_NODE,
        &authorization.root_receipt_sha256,
        &mut visited,
        &mut selected_inputs,
    )?;
    let root_receipt = read_canonical_json(&root_receipt_path, "R1 root receipt")?;
    let root_evidence = object(&root_receipt, "evidence")?;
    if root_evidence
        .get("decisionFreezeSha256")
        .and_then(Value::as_str)
        != Some(authorization.decision_freeze_sha256.as_str())
    {
        bail!("R1 root receipt does not bind the decision freeze");
    }
    if selected_inputs.get("r1DecisionSha256") != Some(&authorization.decision_freeze_sha256) {
        bail!("R1 root selected input does not bind the decision freeze");
    }

    let decision_path = contained_content_file(
        &store_root,
        &store_root
            .join("objects")
            .join(format!("{}.json", authorization.decision_freeze_sha256)),
        "objects",
        "R1 decision freeze",
    )?;
    let decision = read_canonical_json(&decision_path, "R1 decision freeze")?;
    if content_address(&decision_path)? != authorization.decision_freeze_sha256 {
        bail!("R1 decision freeze content address mismatch");
    }
    validate_decision_freeze(&decision, &authorization)?;

    Ok(MaterializationAuthorization {
        store_root,
        store_id: store_id.to_string(),
        authorization_path,
        root_receipt_path,
        authorization_sha256,
        graph_hash: authorization.graph_hash,
        root_receipt_sha256: authorization.root_receipt_sha256,
        decision_freeze_sha256: authorization.decision_freeze_sha256,
        output_path,
        agent_seed: input.agent_seed,
        controller_seed: input.controller_seed,
        series_nonce: input.series_nonce,
        series_id,
        verifying_key: verifying_key.to_bytes(),
    })
}

impl MaterializationAuthorization {
    pub(crate) fn validate_for(&self, options: &MaterializeOptions) -> Result<()> {
        if canonical_absent_output(&options.experiment_root)? != self.output_path {
            bail!("materialization capability is bound to a different output path");
        }
        let verifying_key = VerifyingKey::from_bytes(&self.verifying_key)
            .context("materialization capability verifier is invalid")?;
        let (authorization, authorization_sha256) =
            verify_authorization_envelope(&self.authorization_path, &verifying_key)?;
        if authorization_sha256 != self.authorization_sha256
            || content_address(&self.authorization_path)? != self.authorization_sha256
            || content_address(&self.root_receipt_path)? != self.root_receipt_sha256
        {
            bail!("materialization capability evidence changed after issuance");
        }
        let graph_path = contained_content_file(
            &self.store_root,
            &self
                .store_root
                .join("graphs")
                .join(format!("{}.json", self.graph_hash)),
            "graphs",
            "readiness graph",
        )?;
        let graph = read_canonical_json(&graph_path, "readiness graph")?;
        let (pinned_graph, pinned_graph_hash) = pinned_readiness_graph()?;
        if graph != pinned_graph || self.graph_hash != pinned_graph_hash {
            bail!("materialization capability readiness graph is no longer pinned");
        }
        let nodes = graph_node_specs(&graph)?;
        let mut visited = BTreeMap::new();
        let mut selected_inputs = BTreeMap::new();
        validate_ready_receipt_chain(
            &self.store_root,
            &self.graph_hash,
            &self.store_id,
            &nodes,
            R1_ROOT_NODE,
            &self.root_receipt_sha256,
            &mut visited,
            &mut selected_inputs,
        )?;
        if selected_inputs.get("r1DecisionSha256") != Some(&self.decision_freeze_sha256) {
            bail!("materialization capability decision selected input changed");
        }
        let decision_path = contained_content_file(
            &self.store_root,
            &self
                .store_root
                .join("objects")
                .join(format!("{}.json", self.decision_freeze_sha256)),
            "objects",
            "R1 decision freeze",
        )?;
        if content_address(&decision_path)? != self.decision_freeze_sha256 {
            bail!("materialization decision freeze changed after issuance");
        }
        let decision = read_canonical_json(&decision_path, "R1 decision freeze")?;
        validate_decision_freeze(&decision, &authorization)?;
        Ok(())
    }

    pub(crate) fn derive_slot_seed(&self, base_seed: u64, slot_key: &str) -> u64 {
        let mut digest = Sha256::new();
        digest.update(b"semantic-editing-e04-r1-agent-slot/0.1\0");
        digest.update(self.series_nonce.as_bytes());
        digest.update([0]);
        digest.update(self.agent_seed.as_bytes());
        digest.update([0]);
        digest.update(base_seed.to_be_bytes());
        digest.update([0]);
        digest.update(slot_key.as_bytes());
        u64::from_be_bytes(digest.finalize()[..8].try_into().expect("SHA-256 prefix"))
    }

    pub(crate) fn series_id(&self) -> &str {
        &self.series_id
    }

    pub(crate) fn controller_seed_commitment(&self) -> String {
        let mut digest = Sha256::new();
        digest.update(b"semantic-editing-e04-r1-controller-seed/0.1\0");
        digest.update(self.series_nonce.as_bytes());
        digest.update([0]);
        digest.update(self.controller_seed.as_bytes());
        hex::encode(digest.finalize())
    }

    pub(crate) fn result_binding(&self) -> MaterializationResultBinding {
        MaterializationResultBinding {
            authorization_envelope_sha256: self.authorization_sha256.clone(),
            root_receipt_sha256: self.root_receipt_sha256.clone(),
            decision_freeze_sha256: self.decision_freeze_sha256.clone(),
            series_id: self.series_id.clone(),
            output_path: self.output_path.clone(),
        }
    }
}

pub(crate) struct MaterializationResultBinding {
    pub authorization_envelope_sha256: String,
    pub root_receipt_sha256: String,
    pub decision_freeze_sha256: String,
    pub series_id: String,
    pub output_path: PathBuf,
}

pub(crate) fn canonical_json_bytes(value: &Value) -> Vec<u8> {
    canonical_bytes(value)
}

pub(crate) fn production_verifying_key() -> Result<VerifyingKey> {
    let bytes = hex::decode(R1_AUTHORIZATION_VERIFYING_KEY_HEX)
        .context("pinned E04 issuer verification key is malformed")?;
    let bytes: [u8; 32] = bytes
        .try_into()
        .map_err(|_| anyhow::anyhow!("pinned E04 issuer verification key has wrong length"))?;
    VerifyingKey::from_bytes(&bytes).context("pinned E04 issuer verification key is invalid")
}

pub(crate) fn verify_purpose_signature(
    verifying_key: &VerifyingKey,
    purpose: &str,
    payload: &Value,
    signature_hex: &str,
) -> Result<()> {
    if signature_hex.len() != 128
        || !signature_hex
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        bail!("purpose-bound signature is malformed");
    }
    let signature = Signature::from_slice(
        &hex::decode(signature_hex).context("purpose-bound signature is malformed")?,
    )
    .context("purpose-bound signature is malformed")?;
    let mut bytes = Vec::from(purpose.as_bytes());
    bytes.push(0);
    bytes.extend(canonical_bytes(payload));
    verifying_key
        .verify(&bytes, &signature)
        .context("purpose-bound signature is not trusted")
}

fn authorization_signing_bytes(payload: &SerializedAuthorization) -> Result<Vec<u8>> {
    let mut bytes = Vec::from(R1_AUTHORIZATION_PURPOSE.as_bytes());
    bytes.push(0);
    bytes.extend(canonical_bytes(&serde_json::to_value(payload)?));
    Ok(bytes)
}

fn verify_authorization_envelope(
    path: &Path,
    verifying_key: &VerifyingKey,
) -> Result<(SerializedAuthorization, String)> {
    let value = read_canonical_json(path, "signed materialization authorization")?;
    let envelope: SignedAuthorizationEnvelope =
        serde_json::from_value(value).context("invalid signed materialization authorization")?;
    if envelope.schema != R1_AUTHORIZATION_ENVELOPE_SCHEMA
        || envelope.issuer != R1_AUTHORIZATION_ISSUER
        || envelope.purpose != R1_AUTHORIZATION_PURPOSE
        || envelope.signature.len() != 128
        || !envelope
            .signature
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        bail!("signed materialization authorization issuer/purpose contract mismatch");
    }
    let signature = Signature::from_slice(
        &hex::decode(&envelope.signature).context("materialization signature is malformed")?,
    )
    .context("materialization signature is malformed")?;
    verifying_key
        .verify(&authorization_signing_bytes(&envelope.payload)?, &signature)
        .context("materialization authorization signature is not trusted")?;
    Ok((envelope.payload, sha256(&fs::read(path)?)))
}

fn pinned_readiness_graph() -> Result<(Value, String)> {
    let graph: Value = serde_json::from_slice(PINNED_READINESS_GRAPH)
        .context("embedded readiness graph is malformed")?;
    if string(&graph, "schema")? != READINESS_GRAPH_SCHEMA {
        bail!("embedded readiness graph schema is invalid");
    }
    let canonical = canonical_bytes(&graph);
    Ok((graph, sha256(&canonical)))
}

pub(crate) fn pinned_readiness_contract() -> Result<(String, String)> {
    Ok((
        pinned_readiness_graph()?.1,
        sha256(PINNED_READINESS_CHECKER),
    ))
}

pub(crate) struct VerifiedReadinessRoot {
    pub store_root: PathBuf,
    pub selected_inputs: BTreeMap<String, String>,
    pub receipt_hashes: BTreeMap<String, String>,
}

pub(crate) fn verify_pinned_readiness_root(
    readiness_store: &Path,
    root_receipt_path: &Path,
    expected_store_id: &str,
    expected_graph_hash: &str,
    root_node: &str,
    root_receipt_sha256: &str,
) -> Result<VerifiedReadinessRoot> {
    let store_root = canonical_directory(readiness_store, "readiness store")?;
    for directory in ["objects", "current", "graphs"] {
        canonical_directory(&store_root.join(directory), directory)?;
    }
    let store: Value = read_canonical_json(&store_root.join("STORE.json"), "readiness store")?;
    require_exact_keys(&store, &["schema", "storeId"], "readiness store")?;
    if string(&store, "schema")? != READINESS_STORE_SCHEMA
        || string(&store, "storeId")? != expected_store_id
    {
        bail!("signed readiness store identity mismatch");
    }
    let (pinned_graph, pinned_hash) = pinned_readiness_graph()?;
    if expected_graph_hash != pinned_hash {
        bail!("signed readiness graph is not the pinned graph");
    }
    let graph_path = contained_content_file(
        &store_root,
        &store_root
            .join("graphs")
            .join(format!("{pinned_hash}.json")),
        "graphs",
        "readiness graph",
    )?;
    if content_address(&graph_path)? != pinned_hash
        || read_canonical_json(&graph_path, "readiness graph")? != pinned_graph
    {
        bail!("pinned readiness graph object mismatch");
    }
    let nodes = graph_node_specs(&pinned_graph)?;
    if !nodes.contains_key(root_node) || !graph_roots(&pinned_graph)?.contains(root_node) {
        bail!("signed readiness root is not a pinned graph root");
    }
    let root_path = contained_content_file(
        &store_root,
        root_receipt_path,
        "objects",
        "signed readiness root receipt",
    )?;
    if content_address(&root_path)? != root_receipt_sha256 {
        bail!("signed readiness root receipt content address mismatch");
    }
    let mut visited = BTreeMap::new();
    let mut selected_inputs = BTreeMap::new();
    validate_ready_receipt_chain(
        &store_root,
        &pinned_hash,
        expected_store_id,
        &nodes,
        root_node,
        root_receipt_sha256,
        &mut visited,
        &mut selected_inputs,
    )?;
    Ok(VerifiedReadinessRoot {
        store_root,
        selected_inputs,
        receipt_hashes: visited,
    })
}

fn validate_decision_freeze(value: &Value, authorization: &SerializedAuthorization) -> Result<()> {
    let materialization = object(value, "r1Materialization")?;
    let expected = [
        ("outputPath", authorization.output_path.as_str()),
        ("agentSeedSha256", authorization.agent_seed_sha256.as_str()),
        (
            "controllerSeedSha256",
            authorization.controller_seed_sha256.as_str(),
        ),
        (
            "seriesNonceSha256",
            authorization.series_nonce_sha256.as_str(),
        ),
        ("seriesId", authorization.series_id.as_str()),
    ];
    for (key, expected) in expected {
        if materialization.get(key).and_then(Value::as_str) != Some(expected) {
            bail!("R1 decision freeze materialization binding mismatch: {key}");
        }
    }
    if materialization
        .get("materializerContractSha256")
        .and_then(Value::as_str)
        != Some(materializer_contract_sha256().as_str())
    {
        bail!("R1 decision freeze materializer contract mismatch");
    }
    Ok(())
}

fn validate_ready_receipt_chain(
    store: &Path,
    graph_hash: &str,
    store_id: &str,
    graph: &BTreeMap<String, ReadinessNodeSpec>,
    node: &str,
    receipt_hash: &str,
    visited: &mut BTreeMap<String, String>,
    selected_values: &mut BTreeMap<String, String>,
) -> Result<()> {
    if let Some(previous) = visited.get(node) {
        if previous != receipt_hash {
            bail!("readiness DAG references conflicting receipts for {node}");
        }
        return Ok(());
    }
    visited.insert(node.to_owned(), receipt_hash.to_owned());
    let spec = graph
        .get(node)
        .with_context(|| format!("readiness receipt has unknown node {node}"))?;
    let expected_dependencies = &spec.dependencies;
    let pointer_path = store.join("current").join(format!("{node}.json"));
    let pointer = read_canonical_json(&pointer_path, "readiness pointer")?;
    require_exact_keys(
        &pointer,
        &["schema", "storeId", "graphHash", "node", "receiptHash"],
        "readiness pointer",
    )?;
    if string(&pointer, "schema")? != READINESS_POINTER_SCHEMA
        || string(&pointer, "storeId")? != store_id
        || string(&pointer, "graphHash")? != graph_hash
        || string(&pointer, "node")? != node
        || string(&pointer, "receiptHash")? != receipt_hash
    {
        bail!("current readiness pointer mismatch for {node}");
    }
    let receipt_path = store.join("objects").join(format!("{receipt_hash}.json"));
    let receipt = read_canonical_json(&receipt_path, "readiness receipt")?;
    require_exact_keys(
        &receipt,
        &[
            "schema",
            "storeId",
            "graphHash",
            "checkerVersion",
            "node",
            "nodeKey",
            "status",
            "selectedInputs",
            "dependencies",
            "evidence",
            "error",
            "createdUnixNs",
        ],
        "readiness receipt",
    )?;
    if content_address(&receipt_path)? != receipt_hash
        || string(&receipt, "schema")? != READINESS_RECEIPT_SCHEMA
        || string(&receipt, "storeId")? != store_id
        || string(&receipt, "graphHash")? != graph_hash
        || string(&receipt, "checkerVersion")? != READINESS_CHECKER_VERSION
        || string(&receipt, "node")? != node
        || string(&receipt, "status")? != "READY"
        || receipt.get("error").is_some_and(|value| !value.is_null())
        || receipt
            .get("createdUnixNs")
            .and_then(Value::as_u64)
            .is_none()
        || receipt.get("evidence").and_then(Value::as_object).is_none()
    {
        bail!("readiness receipt is not current READY evidence for {node}");
    }
    let selected = object(&receipt, "selectedInputs")?;
    let selected_keys = selected.keys().cloned().collect::<BTreeSet<_>>();
    let expected_keys = spec
        .input_selectors
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>();
    if selected_keys != expected_keys {
        bail!("readiness selected-input schema mismatch for {node}");
    }
    for (key, value) in selected {
        let value = value
            .as_str()
            .filter(|value| !value.is_empty())
            .with_context(|| format!("readiness selected input is invalid for {node}"))?;
        if let Some(previous) = selected_values.insert(key.clone(), value.to_owned())
            && previous != value
        {
            bail!("readiness closure has conflicting selected input {key}");
        }
    }
    let dependencies = object(&receipt, "dependencies")?;
    if dependencies.len() != expected_dependencies.len()
        || expected_dependencies
            .iter()
            .any(|dependency| !dependencies.contains_key(dependency))
    {
        bail!("readiness receipt dependency closure mismatch for {node}");
    }
    let node_key_value = serde_json::json!({
        "storeId":store_id,
        "graphHash":graph_hash,
        "checkerVersion":READINESS_CHECKER_VERSION,
        "checker":spec.checker.as_str(),
        "checkerSourceSha256":sha256(PINNED_READINESS_CHECKER),
        "node":node,
        "inputs":receipt.get("selectedInputs").expect("validated selected inputs"),
        "dependencies":receipt.get("dependencies").expect("validated dependencies"),
    });
    let expected_node_key = sha256(&canonical_bytes(&node_key_value));
    if string(&receipt, "nodeKey")? != expected_node_key {
        bail!("readiness checker/nodeKey authority mismatch for {node}");
    }
    for dependency in expected_dependencies {
        let dependency_hash = dependencies
            .get(dependency)
            .and_then(Value::as_str)
            .with_context(|| format!("readiness dependency hash missing for {dependency}"))?;
        checked_sha(dependency_hash, "readiness dependency hash")?;
        validate_ready_receipt_chain(
            store,
            graph_hash,
            store_id,
            graph,
            dependency,
            dependency_hash,
            visited,
            selected_values,
        )?;
    }
    Ok(())
}

fn graph_node_specs(value: &Value) -> Result<BTreeMap<String, ReadinessNodeSpec>> {
    let nodes = value
        .get("nodes")
        .and_then(Value::as_array)
        .context("readiness graph nodes missing")?;
    let mut result = BTreeMap::new();
    for node in nodes {
        let id = string(node, "id")?.to_owned();
        let dependencies = node
            .get("dependencies")
            .and_then(Value::as_array)
            .context("readiness graph dependency list missing")?
            .iter()
            .map(|value| {
                value
                    .as_str()
                    .map(str::to_owned)
                    .context("readiness graph dependency must be a string")
            })
            .collect::<Result<Vec<_>>>()?;
        let checker = string(node, "checker")?.to_owned();
        let input_selectors = node
            .get("inputSelectors")
            .and_then(Value::as_array)
            .context("readiness graph input selector list missing")?
            .iter()
            .map(|value| {
                value
                    .as_str()
                    .map(str::to_owned)
                    .context("readiness graph input selector must be a string")
            })
            .collect::<Result<Vec<_>>>()?;
        if result
            .insert(
                id,
                ReadinessNodeSpec {
                    checker,
                    dependencies,
                    input_selectors,
                },
            )
            .is_some()
        {
            bail!("readiness graph contains duplicate node");
        }
    }
    Ok(result)
}

fn graph_roots(value: &Value) -> Result<BTreeSet<String>> {
    value
        .get("roots")
        .and_then(Value::as_array)
        .context("readiness graph roots missing")?
        .iter()
        .map(|root| {
            root.as_str()
                .map(str::to_owned)
                .context("readiness graph root must be a string")
        })
        .collect()
}

pub(crate) fn canonical_absent_output(path: &Path) -> Result<PathBuf> {
    if !path.is_absolute()
        || path
            .components()
            .any(|component| matches!(component, Component::CurDir | Component::ParentDir))
    {
        bail!("materialization output path must be absolute and normalized");
    }
    if fs::symlink_metadata(path).is_ok() {
        bail!("materialization output path must be exactly absent");
    }
    let name = path
        .file_name()
        .context("materialization output has no name")?;
    let parent = path
        .parent()
        .context("materialization output has no parent")?;
    let canonical_parent = canonical_directory(parent, "materialization output parent")?;
    Ok(canonical_parent.join(name))
}

pub(crate) fn canonical_directory(path: &Path, label: &str) -> Result<PathBuf> {
    let metadata = fs::symlink_metadata(path)
        .with_context(|| format!("{label} is missing: {}", path.display()))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        bail!("{label} must be a regular non-symlink directory");
    }
    fs::canonicalize(path).with_context(|| format!("cannot canonicalize {label}"))
}

pub(crate) fn contained_content_file(
    store: &Path,
    path: &Path,
    directory: &str,
    label: &str,
) -> Result<PathBuf> {
    let expected_parent = canonical_directory(&store.join(directory), directory)?;
    let metadata = fs::symlink_metadata(path)
        .with_context(|| format!("{label} is missing: {}", path.display()))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        bail!("{label} must be a regular non-symlink file");
    }
    let canonical = fs::canonicalize(path)?;
    if canonical.parent() != Some(expected_parent.as_path()) {
        bail!("{label} is outside its authority-owned content store");
    }
    Ok(canonical)
}

pub(crate) fn content_address(path: &Path) -> Result<String> {
    let stem = path
        .file_stem()
        .and_then(|value| value.to_str())
        .context("content-addressed file has no UTF-8 hash name")?;
    checked_sha(stem, "content-addressed file name")?;
    let actual = sha256(&fs::read(path)?);
    if actual != stem {
        bail!("content-addressed file digest mismatch");
    }
    Ok(actual)
}

pub(crate) fn read_canonical_json(path: &Path, label: &str) -> Result<Value> {
    let metadata = fs::symlink_metadata(path)
        .with_context(|| format!("{label} is missing: {}", path.display()))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        bail!("{label} must be a regular non-symlink file");
    }
    let raw = fs::read(path)?;
    let value: Value = serde_json::from_slice(&raw).with_context(|| format!("invalid {label}"))?;
    if canonical_bytes(&value) != raw {
        bail!("{label} is not canonical JSON");
    }
    Ok(value)
}

fn canonical_bytes(value: &Value) -> Vec<u8> {
    fn render(value: &Value, output: &mut String) {
        match value {
            Value::Null => output.push_str("null"),
            Value::Bool(value) => output.push_str(if *value { "true" } else { "false" }),
            Value::Number(value) => output.push_str(&value.to_string()),
            Value::String(value) => {
                output.push_str(&serde_json::to_string(value).expect("string serialization"))
            }
            Value::Array(values) => {
                output.push('[');
                for (index, value) in values.iter().enumerate() {
                    if index != 0 {
                        output.push(',');
                    }
                    render(value, output);
                }
                output.push(']');
            }
            Value::Object(values) => {
                output.push('{');
                let mut entries = values.iter().collect::<Vec<_>>();
                entries.sort_by(|left, right| left.0.cmp(right.0));
                for (index, (key, value)) in entries.into_iter().enumerate() {
                    if index != 0 {
                        output.push(',');
                    }
                    output.push_str(&serde_json::to_string(key).expect("key serialization"));
                    output.push(':');
                    render(value, output);
                }
                output.push('}');
            }
        }
    }
    let mut output = String::new();
    render(value, &mut output);
    output.push('\n');
    output.into_bytes()
}

fn require_exact_keys(value: &Value, keys: &[&str], label: &str) -> Result<()> {
    let actual = value
        .as_object()
        .with_context(|| format!("{label} must be an object"))?
        .keys()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    let expected = keys.iter().copied().collect::<BTreeSet<_>>();
    if actual != expected {
        bail!("{label} has an unexpected schema");
    }
    Ok(())
}

fn object<'a>(value: &'a Value, key: &str) -> Result<&'a serde_json::Map<String, Value>> {
    value
        .get(key)
        .and_then(Value::as_object)
        .with_context(|| format!("missing object field {key}"))
}

fn string<'a>(value: &'a Value, key: &str) -> Result<&'a str> {
    value
        .get(key)
        .and_then(Value::as_str)
        .with_context(|| format!("missing string field {key}"))
}

fn checked_sha<'a>(value: &'a str, label: &str) -> Result<&'a str> {
    if value.len() != HEX_SHA256_LEN
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        bail!("{label} must be lowercase SHA-256 hex");
    }
    Ok(value)
}

fn validate_secret(label: &str, value: &str) -> Result<()> {
    if value.is_empty() || value.len() > MAX_SECRET_BYTES {
        bail!("{label} must contain 1..={MAX_SECRET_BYTES} UTF-8 bytes");
    }
    Ok(())
}

fn derive_series_id(agent_seed: &[u8], controller_seed: &[u8], series_nonce: &[u8]) -> String {
    let mut digest = Sha256::new();
    digest.update(b"semantic-editing-e04-r1-series/0.1\0");
    digest.update(series_nonce);
    digest.update([0]);
    digest.update(agent_seed);
    digest.update([0]);
    digest.update(controller_seed);
    hex::encode(digest.finalize())
}

fn sha256(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};
    use serde_json::json;

    struct Fixture {
        _temporary: tempfile::TempDir,
        store: PathBuf,
        authorization: PathBuf,
        root_receipt: PathBuf,
        output: PathBuf,
        agent_seed: String,
        controller_seed: String,
        series_nonce: String,
        signing_key: [u8; 32],
        payload: SerializedAuthorization,
    }

    fn authorize_fixture(fixture: &Fixture) -> Result<MaterializationAuthorization> {
        authorize_materialization_with_verifier(
            fixture.input(),
            SigningKey::from_bytes(&fixture.signing_key).verifying_key(),
        )
    }

    impl Fixture {
        fn input(&self) -> MaterializationAuthorizationInput {
            MaterializationAuthorizationInput {
                readiness_store: self.store.clone(),
                authorization_path: self.authorization.clone(),
                root_receipt_path: self.root_receipt.clone(),
                output_path: self.output.clone(),
                agent_seed: self.agent_seed.clone(),
                controller_seed: self.controller_seed.clone(),
                series_nonce: self.series_nonce.clone(),
            }
        }
    }

    fn write_canonical(path: &Path, value: &Value) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, canonical_bytes(value)).unwrap();
    }

    fn write_object(store: &Path, value: Value) -> (String, PathBuf) {
        let bytes = canonical_bytes(&value);
        let identity = sha256(&bytes);
        let path = store.join("objects").join(format!("{identity}.json"));
        fs::write(&path, bytes).unwrap();
        (identity, path)
    }

    fn write_pointer(store: &Path, store_id: &str, graph_hash: &str, node: &str, hash: &str) {
        write_canonical(
            &store.join("current").join(format!("{node}.json")),
            &json!({
                "schema":READINESS_POINTER_SCHEMA,"storeId":store_id,
                "graphHash":graph_hash,"node":node,"receiptHash":hash
            }),
        );
    }

    #[derive(Clone, Copy, PartialEq, Eq)]
    enum ReceiptMutation {
        None,
        CheckerVersion,
        NodeKey,
        SelectedInput,
        ConflictingClosureInput,
    }

    fn receipt_chain(
        store: &Path,
        store_id: &str,
        graph_hash: &str,
        specs: &BTreeMap<String, ReadinessNodeSpec>,
        node: &str,
        decision_hash: &str,
        mutation: ReceiptMutation,
        written: &mut BTreeMap<String, (String, PathBuf)>,
        created: &mut u64,
    ) -> (String, PathBuf) {
        if let Some(value) = written.get(node) {
            return value.clone();
        }
        let spec = specs.get(node).unwrap();
        let dependencies = spec
            .dependencies
            .iter()
            .map(|dependency| {
                let (hash, _) = receipt_chain(
                    store,
                    store_id,
                    graph_hash,
                    specs,
                    dependency,
                    decision_hash,
                    mutation,
                    written,
                    created,
                );
                (dependency.clone(), Value::String(hash))
            })
            .collect::<serde_json::Map<_, _>>();
        let mut selected = spec
            .input_selectors
            .iter()
            .map(|selector| {
                let value = if selector == "r1DecisionSha256" {
                    decision_hash.to_owned()
                } else {
                    sha256(selector.as_bytes())
                };
                (selector.clone(), Value::String(value))
            })
            .collect::<serde_json::Map<_, _>>();
        if mutation == ReceiptMutation::SelectedInput && node == R1_ROOT_NODE {
            selected.insert("r1DecisionSha256".into(), Value::String("7".repeat(64)));
        }
        if mutation == ReceiptMutation::ConflictingClosureInput
            && node == "DIAGNOSTIC_PREFLIGHT_READY"
        {
            selected.insert(
                "diagnosticPublicSetSha256".into(),
                Value::String("6".repeat(64)),
            );
        }
        let node_key_material = json!({
            "storeId":store_id,"graphHash":graph_hash,
            "checkerVersion":READINESS_CHECKER_VERSION,"checker":spec.checker.as_str(),
            "checkerSourceSha256":sha256(PINNED_READINESS_CHECKER),"node":node,
            "inputs":&selected,"dependencies":&dependencies
        });
        let mut node_key = sha256(&canonical_bytes(&node_key_material));
        if mutation == ReceiptMutation::NodeKey && node == "R1_DECISION_FREEZE_VERIFY" {
            node_key = "5".repeat(64);
        }
        let checker_version =
            if mutation == ReceiptMutation::CheckerVersion && node == "R1_DECISION_FREEZE_VERIFY" {
                "attacker-checker/0.1"
            } else {
                READINESS_CHECKER_VERSION
            };
        *created += 1;
        let evidence = if node == R1_ROOT_NODE {
            json!({"decisionFreezeSha256":decision_hash})
        } else {
            json!({})
        };
        let receipt = json!({
            "schema":READINESS_RECEIPT_SCHEMA,"storeId":store_id,"graphHash":graph_hash,
            "checkerVersion":checker_version,"node":node,"nodeKey":node_key,
            "status":"READY","selectedInputs":selected,"dependencies":dependencies,
            "evidence":evidence,"error":null,"createdUnixNs":*created
        });
        let result = write_object(store, receipt);
        write_pointer(store, store_id, graph_hash, node, &result.0);
        written.insert(node.to_owned(), result.clone());
        result
    }

    fn fixture() -> Fixture {
        fixture_with(ReceiptMutation::None)
    }

    fn fixture_with(mutation: ReceiptMutation) -> Fixture {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path();
        let store = root.join("store");
        for directory in ["objects", "current", "graphs", "authorizations"] {
            fs::create_dir_all(store.join(directory)).unwrap();
        }
        let store_id = "1".repeat(64);
        write_canonical(
            &store.join("STORE.json"),
            &json!({"schema":READINESS_STORE_SCHEMA,"storeId":store_id}),
        );

        let (graph, graph_hash) = pinned_readiness_graph().unwrap();
        let graph_bytes = canonical_bytes(&graph);
        fs::write(
            store.join("graphs").join(format!("{graph_hash}.json")),
            graph_bytes,
        )
        .unwrap();

        let agent_seed = "agent-secret-r1".to_owned();
        let controller_seed = "controller-secret-r1".to_owned();
        let series_nonce = "fresh-post-canary-series".to_owned();
        let series_id = derive_series_id(
            agent_seed.as_bytes(),
            controller_seed.as_bytes(),
            series_nonce.as_bytes(),
        );
        let output = fs::canonicalize(root).unwrap().join("r1-experiment");
        let decision = json!({
            "schema":"semantic-editing-e04-r1-decision-freeze/0.1",
            "r1Materialization":{
                "outputPath":output.to_string_lossy(),
                "agentSeedSha256":sha256(agent_seed.as_bytes()),
                "controllerSeedSha256":sha256(controller_seed.as_bytes()),
                "seriesNonceSha256":sha256(series_nonce.as_bytes()),
                "seriesId":series_id,
                "materializerContractSha256":materializer_contract_sha256()
            }
        });
        let (decision_hash, _) = write_object(&store, decision);
        let specs = graph_node_specs(&graph).unwrap();
        let mut written = BTreeMap::new();
        let mut created = 0;
        let (root_hash, root_receipt) = receipt_chain(
            &store,
            &store_id,
            &graph_hash,
            &specs,
            R1_ROOT_NODE,
            &decision_hash,
            mutation,
            &mut written,
            &mut created,
        );
        let authorization = SerializedAuthorization {
            schema: R1_AUTHORIZATION_SCHEMA.into(),
            store_id,
            graph_hash,
            root_node: R1_ROOT_NODE.into(),
            root_receipt_sha256: root_hash,
            decision_freeze_sha256: decision_hash,
            output_path: output.to_string_lossy().into_owned(),
            agent_seed_sha256: sha256(agent_seed.as_bytes()),
            controller_seed_sha256: sha256(controller_seed.as_bytes()),
            series_nonce_sha256: sha256(series_nonce.as_bytes()),
            series_id,
            materializer: materializer_identity(),
        };
        let signing_key = [42; 32];
        let key = SigningKey::from_bytes(&signing_key);
        let signature = key.sign(&authorization_signing_bytes(&authorization).unwrap());
        let envelope = SignedAuthorizationEnvelope {
            schema: R1_AUTHORIZATION_ENVELOPE_SCHEMA.into(),
            issuer: R1_AUTHORIZATION_ISSUER.into(),
            purpose: R1_AUTHORIZATION_PURPOSE.into(),
            payload: authorization.clone(),
            signature: hex::encode(signature.to_bytes()),
        };
        let authorization_bytes = canonical_bytes(&serde_json::to_value(envelope).unwrap());
        let authorization_hash = sha256(&authorization_bytes);
        let authorization_path = store
            .join("authorizations")
            .join(format!("{authorization_hash}.json"));
        fs::write(&authorization_path, authorization_bytes).unwrap();

        Fixture {
            _temporary: temporary,
            store,
            authorization: authorization_path,
            root_receipt,
            output,
            agent_seed,
            controller_seed,
            series_nonce,
            signing_key,
            payload: authorization,
        }
    }

    fn install_envelope(
        fixture: &mut Fixture,
        payload: SerializedAuthorization,
        signing_key: [u8; 32],
        signing_purpose: &str,
    ) {
        let mut signing_bytes = Vec::from(signing_purpose.as_bytes());
        signing_bytes.push(0);
        signing_bytes.extend(canonical_bytes(&serde_json::to_value(&payload).unwrap()));
        let signature = SigningKey::from_bytes(&signing_key).sign(&signing_bytes);
        let envelope = SignedAuthorizationEnvelope {
            schema: R1_AUTHORIZATION_ENVELOPE_SCHEMA.into(),
            issuer: R1_AUTHORIZATION_ISSUER.into(),
            purpose: R1_AUTHORIZATION_PURPOSE.into(),
            payload: payload.clone(),
            signature: hex::encode(signature.to_bytes()),
        };
        let bytes = canonical_bytes(&serde_json::to_value(envelope).unwrap());
        let hash = sha256(&bytes);
        let path = fixture
            .store
            .join("authorizations")
            .join(format!("{hash}.json"));
        fs::write(&path, bytes).unwrap();
        fixture.authorization = path;
        fixture.payload = payload;
    }

    #[test]
    fn r1_materialization_authorization_accepts_exact_current_evidence() {
        let fixture = fixture();
        let authorization = authorize_fixture(&fixture).unwrap();
        assert_eq!(authorization.series_id().len(), 64);
        assert_eq!(
            authorization.derive_slot_seed(7, "slot"),
            authorization.derive_slot_seed(7, "slot")
        );
        assert_ne!(authorization.derive_slot_seed(7, "slot"), 7);
    }

    #[test]
    fn r1_materialization_capability_revalidates_the_transitive_chain_before_use() {
        let fixture = fixture();
        let authorization = authorize_fixture(&fixture).unwrap();
        let canary_pointer = fixture
            .store
            .join("current")
            .join("DIAGNOSTIC_CANARY_3_COMPLETE.json");
        let mut pointer: Value =
            serde_json::from_slice(&fs::read(&canary_pointer).unwrap()).unwrap();
        pointer["receiptHash"] = Value::String("8".repeat(64));
        write_canonical(&canary_pointer, &pointer);
        let options = MaterializeOptions {
            experiment_root: fixture.output,
            population_json: include_str!(
                "../../../benchmarks/semantic-change/editing-population-v1.json"
            )
            .into(),
            binder_freeze: FROZEN_PRODUCT_REVISION.into(),
            binder_tree_sha256: FROZEN_BINDER_TREE_SHA256.into(),
            population_sha256: FROZEN_POPULATION_SHA256.into(),
            gradle_wrapper_assets: None,
        };
        assert!(authorization.validate_for(&options).is_err());
    }

    #[test]
    fn r1_materialization_authorization_refuses_forged_cross_store_and_stale_root() {
        let forged = fixture();
        fs::write(&forged.authorization, b"{}\n").unwrap();
        assert!(authorize_fixture(&forged).is_err());

        let cross_store = fixture();
        let other = cross_store._temporary.path().join("other-store");
        fs::create_dir(&other).unwrap();
        let mut input = cross_store.input();
        input.readiness_store = other;
        assert!(
            authorize_materialization_with_verifier(
                input,
                SigningKey::from_bytes(&cross_store.signing_key).verifying_key()
            )
            .is_err()
        );

        let stale = fixture();
        let pointer_path = stale
            .store
            .join("current")
            .join(format!("{R1_ROOT_NODE}.json"));
        let mut pointer: Value = serde_json::from_slice(&fs::read(&pointer_path).unwrap()).unwrap();
        pointer["receiptHash"] = Value::String("9".repeat(64));
        write_canonical(&pointer_path, &pointer);
        assert!(authorize_fixture(&stale).is_err());
    }

    #[test]
    fn r1_materialization_authorization_refuses_output_seed_and_symlink_changes() {
        let output = fixture();
        fs::create_dir(&output.output).unwrap();
        assert!(authorize_fixture(&output).is_err());

        let seed = fixture();
        let mut input = seed.input();
        input.agent_seed.push_str("-changed");
        assert!(
            authorize_materialization_with_verifier(
                input,
                SigningKey::from_bytes(&seed.signing_key).verifying_key()
            )
            .is_err()
        );

        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;
            let linked = fixture();
            let real = linked.authorization.with_extension("real");
            fs::rename(&linked.authorization, &real).unwrap();
            symlink(&real, &linked.authorization).unwrap();
            assert!(authorize_fixture(&linked).is_err());

            let linked_root = fixture();
            let real = linked_root.root_receipt.with_extension("real");
            fs::rename(&linked_root.root_receipt, &real).unwrap();
            symlink(&real, &linked_root.root_receipt).unwrap();
            assert!(authorize_fixture(&linked_root).is_err());
        }
    }

    #[test]
    fn r1_materialization_refuses_unsigned_self_signed_and_cross_purpose_envelopes() {
        let mut unsigned = fixture();
        let envelope = SignedAuthorizationEnvelope {
            schema: R1_AUTHORIZATION_ENVELOPE_SCHEMA.into(),
            issuer: R1_AUTHORIZATION_ISSUER.into(),
            purpose: R1_AUTHORIZATION_PURPOSE.into(),
            payload: unsigned.payload.clone(),
            signature: "0".repeat(128),
        };
        let bytes = canonical_bytes(&serde_json::to_value(envelope).unwrap());
        let hash = sha256(&bytes);
        unsigned.authorization = unsigned
            .store
            .join("authorizations")
            .join(format!("{hash}.json"));
        fs::write(&unsigned.authorization, bytes).unwrap();
        assert!(authorize_fixture(&unsigned).is_err());

        let mut self_signed = fixture();
        let payload = self_signed.payload.clone();
        install_envelope(
            &mut self_signed,
            payload,
            [99; 32],
            R1_AUTHORIZATION_PURPOSE,
        );
        assert!(authorize_fixture(&self_signed).is_err());

        let mut reflected = fixture();
        let payload = reflected.payload.clone();
        let signing_key = reflected.signing_key;
        install_envelope(
            &mut reflected,
            payload,
            signing_key,
            "codeclew/e04/external-spec/0.1",
        );
        assert!(authorize_fixture(&reflected).is_err());
    }

    #[test]
    fn r1_materialization_refuses_signed_alternate_authority_bindings() {
        for mutate in ["store", "graph", "root", "receipt"] {
            let mut fixture = fixture();
            let mut payload = fixture.payload.clone();
            match mutate {
                "store" => payload.store_id = "2".repeat(64),
                "graph" => payload.graph_hash = "3".repeat(64),
                "root" => payload.root_node = "R1_OTHER_ROOT".into(),
                "receipt" => payload.root_receipt_sha256 = "4".repeat(64),
                _ => unreachable!(),
            }
            let signing_key = fixture.signing_key;
            install_envelope(&mut fixture, payload, signing_key, R1_AUTHORIZATION_PURPOSE);
            assert!(authorize_fixture(&fixture).is_err(), "accepted {mutate}");
        }
    }

    #[test]
    fn r1_materialization_refuses_checker_node_key_selected_input_and_closure_forgery() {
        for mutation in [
            ReceiptMutation::CheckerVersion,
            ReceiptMutation::NodeKey,
            ReceiptMutation::SelectedInput,
            ReceiptMutation::ConflictingClosureInput,
        ] {
            let fixture = fixture_with(mutation);
            assert!(authorize_fixture(&fixture).is_err());
        }
    }
}
