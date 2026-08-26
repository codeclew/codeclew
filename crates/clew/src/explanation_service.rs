//! Managed publication and retained loading for validated explanation bundles.

use crate::canonical;
use crate::cas::{CasObject, CasStore};
use crate::error::{ClewError, ErrorCode};
use crate::explanation::{
    ClaimInputDocument, EXPLANATION_BUNDLE_SCHEMA, ExplanationBundle, ExplanationProjection,
    MAX_EXPLANATION_BUNDLE_BYTES, MAX_EXPLANATION_STDOUT_BYTES, PreparedExplanation,
};
use crate::state::StateAuthority;
use crate::thread::ThreadAuthority;
use crate::thread_flow_service::{self, ThreadFlowRoot};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::path::Path;

pub const EXPLANATION_ROOT_SCHEMA: &str = "codeclew-thread-explanation-root/0.1";
pub const EXPLANATION_RESULT_SCHEMA: &str = "codeclew-thread-explanation-result/0.1";
const MAX_EXPLANATION_ROOT_BYTES: usize = 256 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExplanationRoot {
    pub schema: String,
    pub explanation_id: String,
    pub thread_id: String,
    pub thread_authority_digest: String,
    pub flow_id: String,
    pub flow_slice_ref: CasObject,
    pub bundle_ref: CasObject,
    pub projection: ExplanationProjection,
}

pub fn create(
    thread: &ThreadAuthority,
    flow_id: &str,
    document: ClaimInputDocument,
) -> Result<ExplanationRoot, ClewError> {
    let state = StateAuthority::process_default()?;
    create_with_state(&state, thread, flow_id, document)
}

pub(crate) fn create_with_state(
    state: &StateAuthority,
    thread: &ThreadAuthority,
    flow_id: &str,
    document: ClaimInputDocument,
) -> Result<ExplanationRoot, ClewError> {
    thread.verify()?;
    thread.require_open_with_state(state)?;
    let store = CasStore::open(state)?;
    let (flow_root, _fact_set, flow) =
        thread_flow_service::load_verified(state, &store, thread, flow_id)?;
    let prepared = crate::explanation::build(&flow.slice, &flow.slice_ref, document)?;
    crate::explanation::verify_prepared(&flow.slice, &prepared)?;
    let root = root_from_prepared(thread, &flow_root, &prepared)?;
    bounded_stdout(&root)?;

    let _admission = thread.admit_with_state(state)?;
    crate::thread::revalidate_authority_record(state, thread)?;
    thread_flow_service::revalidate_root_record(state, thread, &flow_root)?;
    publish_prepared(state, &store, thread, &prepared, &root)?;
    Ok(root)
}

pub fn load(thread: &ThreadAuthority, explanation_id: &str) -> Result<ExplanationRoot, ClewError> {
    let state = StateAuthority::process_default()?;
    let store = CasStore::open(&state)?;
    load_verified(&state, &store, thread, explanation_id).map(|(root, _, _)| root)
}

pub(crate) fn load_verified(
    state: &StateAuthority,
    store: &CasStore,
    thread: &ThreadAuthority,
    explanation_id: &str,
) -> Result<(ExplanationRoot, ThreadFlowRoot, PreparedExplanation), ClewError> {
    thread.verify()?;
    let path = explanation_root_path(state, thread, explanation_id)?;
    let bytes = state
        .read_private_file(&path, MAX_EXPLANATION_ROOT_BYTES)
        .map_err(|_| invalid("explanation root is missing or exceeds 256 KiB"))?;
    let root: ExplanationRoot =
        serde_json::from_slice(&bytes).map_err(|_| corrupt("explanation root is invalid"))?;
    if canonical::bytes(&root).map_err(internal)? != bytes
        || root.explanation_id != explanation_id
        || root.thread_id != thread.thread_id
        || root.thread_authority_digest != thread.authority_digest
    {
        return Err(corrupt("explanation root authority is invalid"));
    }
    validate_root(&root)?;
    let (flow_root, _fact_set, flow) =
        thread_flow_service::load_verified(state, store, thread, &root.flow_id)?;
    if flow_root.slice_ref != root.flow_slice_ref {
        return Err(corrupt("explanation parent flow authority changed"));
    }
    let size = usize::try_from(root.bundle_ref.size)
        .map_err(|_| budget("explanation bundle exceeds host size"))?;
    if size > MAX_EXPLANATION_BUNDLE_BYTES {
        return Err(budget("explanation bundle exceeds 16 MiB"));
    }
    let lease = store.read(&root.bundle_ref, size)?;
    let bundle_bytes = lease.bytes().to_vec();
    let bundle: ExplanationBundle = serde_json::from_slice(&bundle_bytes)
        .map_err(|_| corrupt("explanation bundle is invalid"))?;
    if canonical::bytes(&bundle).map_err(internal)? != bundle_bytes {
        return Err(corrupt("explanation bundle is not canonical"));
    }
    let prepared = PreparedExplanation {
        bundle,
        bundle_bytes,
        bundle_ref: root.bundle_ref.clone(),
        projection: root.projection.clone(),
    };
    crate::explanation::verify_prepared(&flow.slice, &prepared)?;
    Ok((root, flow_root, prepared))
}

pub fn bounded_stdout(root: &ExplanationRoot) -> Result<Value, ClewError> {
    validate_root(root)?;
    let value = json!({
        "schema": EXPLANATION_RESULT_SCHEMA,
        "threadId": root.thread_id,
        "threadAuthorityDigest": root.thread_authority_digest,
        "flowId": root.flow_id,
        "explanationId": root.explanation_id,
        "bundleRef": root.bundle_ref,
        "explanation": root.projection,
    });
    if canonical::bytes(&value)
        .map_err(internal)?
        .len()
        .saturating_add(1)
        > MAX_EXPLANATION_STDOUT_BYTES
    {
        return Err(budget("explanation stdout exceeds 64 KiB"));
    }
    Ok(value)
}

fn root_from_prepared(
    thread: &ThreadAuthority,
    flow: &ThreadFlowRoot,
    prepared: &PreparedExplanation,
) -> Result<ExplanationRoot, ClewError> {
    let root = ExplanationRoot {
        schema: EXPLANATION_ROOT_SCHEMA.into(),
        explanation_id: prepared.bundle.explanation_id.clone(),
        thread_id: thread.thread_id.clone(),
        thread_authority_digest: thread.authority_digest.clone(),
        flow_id: flow.flow_id.clone(),
        flow_slice_ref: flow.slice_ref.clone(),
        bundle_ref: prepared.bundle_ref.clone(),
        projection: prepared.projection.clone(),
    };
    validate_root(&root)?;
    Ok(root)
}

fn validate_root(root: &ExplanationRoot) -> Result<(), ClewError> {
    validate_explanation_id(&root.explanation_id)?;
    if root.schema != EXPLANATION_ROOT_SCHEMA
        || root.bundle_ref.object_schema != EXPLANATION_BUNDLE_SCHEMA
        || root.projection.explanation_id != root.explanation_id
        || root.projection.flow_id != root.flow_id
    {
        return Err(corrupt("explanation retained root authority is invalid"));
    }
    Ok(())
}

fn publish_prepared(
    state: &StateAuthority,
    store: &CasStore,
    thread: &ThreadAuthority,
    prepared: &PreparedExplanation,
    root: &ExplanationRoot,
) -> Result<(), ClewError> {
    let path = explanation_root_path(state, thread, &root.explanation_id)?;
    let root_bytes = canonical::bytes(root).map_err(internal)?;
    if state.private_file_exists(&path)? {
        let existing = state.read_private_file(&path, MAX_EXPLANATION_ROOT_BYTES)?;
        if existing != root_bytes {
            return Err(ClewError::new(
                ErrorCode::BindingChanged,
                "explanation identifier is already bound to another bundle",
            ));
        }
        store.read(&prepared.bundle_ref, prepared.bundle_bytes.len())?;
        return Ok(());
    }
    let published = store.put(EXPLANATION_BUNDLE_SCHEMA, &prepared.bundle_bytes)?;
    if published != prepared.bundle_ref {
        return Err(corrupt(
            "CAS publication returned another explanation identity",
        ));
    }
    write_json_create_new(state, &path, root)
}

fn explanation_root_path(
    state: &StateAuthority,
    thread: &ThreadAuthority,
    explanation_id: &str,
) -> Result<std::path::PathBuf, ClewError> {
    let digest = validate_explanation_id(explanation_id)?;
    let directory = state.thread_root(&thread.thread_id)?.join("explanations");
    state.directory_at(&directory)?;
    Ok(directory.join(format!("{digest}.json")))
}

fn validate_explanation_id(value: &str) -> Result<&str, ClewError> {
    value
        .strip_prefix("thread-explanation:sha256:")
        .filter(|digest| {
            digest.len() == 64
                && digest
                    .bytes()
                    .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        })
        .ok_or_else(|| invalid("explanation id is invalid"))
}

fn write_json_create_new<T: Serialize>(
    state: &StateAuthority,
    path: &Path,
    value: &T,
) -> Result<(), ClewError> {
    let relative = path
        .strip_prefix(state.root())
        .map_err(|_| invalid("explanation root escapes managed state"))?;
    let parent = relative
        .parent()
        .ok_or_else(|| invalid("explanation root has no parent"))?;
    let name = relative
        .file_name()
        .ok_or_else(|| invalid("explanation root has no file name"))?;
    let bytes = canonical::bytes(value).map_err(internal)?;
    let directory = state.directory(parent)?;
    if directory.atomic_create(name, &bytes)? {
        return Ok(());
    }
    if directory.read_file(name, MAX_EXPLANATION_ROOT_BYTES)? == bytes {
        Ok(())
    } else {
        Err(ClewError::new(
            ErrorCode::BindingChanged,
            "explanation root was concurrently bound to another bundle",
        ))
    }
}

fn invalid(message: impl Into<String>) -> ClewError {
    ClewError::new(ErrorCode::InvalidInput, message)
}

fn budget(message: impl Into<String>) -> ClewError {
    ClewError::new(ErrorCode::SliceBudgetExceeded, message)
}

fn corrupt(message: impl Into<String>) -> ClewError {
    ClewError::new(ErrorCode::StateCorrupt, message)
}

fn internal(error: impl std::fmt::Display) -> ClewError {
    ClewError::new(ErrorCode::Internal, error.to_string())
}
