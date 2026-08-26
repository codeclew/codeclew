//! Retained two-sided publication for immutable explanation freshness reports.

use crate::canonical;
use crate::cas::{CasLease, CasObject, CasStore};
use crate::error::{ClewError, ErrorCode};
use crate::explanation_freshness::{
    EXPLANATION_FRESHNESS_REPORT_SCHEMA, ExplanationFreshnessProjection, FreshnessSide,
    MAX_FRESHNESS_REPORT_BYTES, MAX_FRESHNESS_STDOUT_BYTES, PreparedExplanationFreshness,
};
use crate::state::StateAuthority;
use crate::thread::{ThreadAuthority, load_with_state as load_thread_with_state};
use crate::thread_change_set::MemberCorrespondence;
use crate::{explanation_service, thread_callables_service, thread_flow_service};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::path::Path;

pub const EXPLANATION_FRESHNESS_ROOT_SCHEMA: &str = "codeclew-explanation-freshness-root/0.1";
pub const EXPLANATION_FRESHNESS_RESULT_SCHEMA: &str = "codeclew-explanation-freshness-result/0.1";
const MAX_FRESHNESS_ROOT_BYTES: usize = 256 * 1024;

#[derive(Debug, Clone)]
pub struct ExplanationFreshnessServiceRequest {
    pub old_explanation_id: String,
    pub against_fact_set_id: String,
    pub against_flow_id: String,
    pub member_correspondence: Vec<MemberCorrespondence>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExplanationFreshnessRoot {
    pub schema: String,
    pub freshness_id: String,
    pub old_thread_id: String,
    pub old_thread_authority_digest: String,
    pub against_thread_id: String,
    pub against_thread_authority_digest: String,
    pub report_ref: CasObject,
    pub projection: ExplanationFreshnessProjection,
}

pub fn create(
    old_thread: &ThreadAuthority,
    against_thread: &ThreadAuthority,
    request: ExplanationFreshnessServiceRequest,
) -> Result<ExplanationFreshnessRoot, ClewError> {
    let state = StateAuthority::process_default()?;
    create_with_state(&state, old_thread, against_thread, request)
}

pub(crate) fn create_with_state(
    state: &StateAuthority,
    old_thread: &ThreadAuthority,
    against_thread: &ThreadAuthority,
    request: ExplanationFreshnessServiceRequest,
) -> Result<ExplanationFreshnessRoot, ClewError> {
    old_thread.verify()?;
    against_thread.verify()?;
    against_thread.require_open_with_state(state)?;
    let store = CasStore::open(state)?;
    let (old_explanation_root, old_flow_root, old_flow, explanation) =
        explanation_service::load_verified(state, &store, old_thread, &request.old_explanation_id)?;
    let (_verified_old_flow_root, old_facts, verified_old_flow) =
        thread_flow_service::load_verified(state, &store, old_thread, &old_flow_root.flow_id)?;
    if verified_old_flow != old_flow {
        return Err(corrupt(
            "old explanation flow was not loaded deterministically",
        ));
    }
    let (old_callable_root, _) = thread_callables_service::load_verified(
        state,
        &store,
        old_thread,
        &old_flow_root.fact_set_id,
    )?;
    let (against_flow_root, against_facts, against_flow) = thread_flow_service::load_verified(
        state,
        &store,
        against_thread,
        &request.against_flow_id,
    )?;
    if against_flow_root.fact_set_id != request.against_fact_set_id {
        return Err(invalid(
            "--against-fact-set does not bind the retained against flow",
        ));
    }
    let (against_callable_root, _) = thread_callables_service::load_verified(
        state,
        &store,
        against_thread,
        &against_flow_root.fact_set_id,
    )?;
    let prepared = crate::explanation_freshness::build(
        FreshnessSide {
            thread_id: &old_thread.thread_id,
            thread_authority_digest: &old_thread.authority_digest,
            fact_set: &old_facts,
            flow: &old_flow,
        },
        &explanation,
        FreshnessSide {
            thread_id: &against_thread.thread_id,
            thread_authority_digest: &against_thread.authority_digest,
            fact_set: &against_facts,
            flow: &against_flow,
        },
        request.member_correspondence,
    )?;
    crate::explanation_freshness::verify_prepared(
        FreshnessSide {
            thread_id: &old_thread.thread_id,
            thread_authority_digest: &old_thread.authority_digest,
            fact_set: &old_facts,
            flow: &old_flow,
        },
        &explanation,
        FreshnessSide {
            thread_id: &against_thread.thread_id,
            thread_authority_digest: &against_thread.authority_digest,
            fact_set: &against_facts,
            flow: &against_flow,
        },
        &prepared,
    )?;
    let root = root_from_prepared(old_thread, against_thread, &prepared)?;
    bounded_stdout(&root)?;
    let _leases = verify_closure(&store, &prepared, false)?;

    let _admission = against_thread.admit_with_state(state)?;
    crate::thread::revalidate_authority_record(state, old_thread)?;
    crate::thread::revalidate_authority_record(state, against_thread)?;
    explanation_service::revalidate_root_record(state, old_thread, &old_explanation_root)?;
    thread_flow_service::revalidate_root_record(state, old_thread, &old_flow_root)?;
    thread_flow_service::revalidate_root_record(state, against_thread, &against_flow_root)?;
    thread_callables_service::revalidate_root_record(state, old_thread, &old_callable_root)?;
    thread_callables_service::revalidate_root_record(
        state,
        against_thread,
        &against_callable_root,
    )?;
    publish_prepared(state, &store, against_thread, &prepared, &root)?;
    Ok(root)
}

pub fn load(
    against_thread: &ThreadAuthority,
    freshness_id: &str,
) -> Result<ExplanationFreshnessRoot, ClewError> {
    let state = StateAuthority::process_default()?;
    let store = CasStore::open(&state)?;
    load_verified(&state, &store, against_thread, freshness_id).map(|(root, _)| root)
}

fn load_verified(
    state: &StateAuthority,
    store: &CasStore,
    against_thread: &ThreadAuthority,
    freshness_id: &str,
) -> Result<(ExplanationFreshnessRoot, PreparedExplanationFreshness), ClewError> {
    against_thread.verify()?;
    let path = freshness_root_path(state, against_thread, freshness_id)?;
    let bytes = state
        .read_private_file(&path, MAX_FRESHNESS_ROOT_BYTES)
        .map_err(|_| invalid("freshness root is missing or exceeds 256 KiB"))?;
    let root: ExplanationFreshnessRoot = serde_json::from_slice(&bytes)
        .map_err(|_| corrupt("freshness retained root is invalid"))?;
    if canonical::bytes(&root).map_err(internal)? != bytes
        || root.freshness_id != freshness_id
        || root.against_thread_id != against_thread.thread_id
        || root.against_thread_authority_digest != against_thread.authority_digest
    {
        return Err(corrupt("freshness retained root authority is invalid"));
    }
    validate_root(&root)?;
    let (old_thread, _) = load_thread_with_state(state, &root.old_thread_id)?;
    if old_thread.authority_digest != root.old_thread_authority_digest {
        return Err(corrupt("freshness old-thread authority changed"));
    }
    let report_size = usize::try_from(root.report_ref.size)
        .map_err(|_| budget("freshness report exceeds host size"))?;
    if report_size > MAX_FRESHNESS_REPORT_BYTES {
        return Err(budget("freshness report exceeds 16 MiB"));
    }
    let report_lease = store.read(&root.report_ref, report_size)?;
    let report: crate::explanation_freshness::ExplanationFreshnessReport =
        serde_json::from_slice(report_lease.bytes())
            .map_err(|_| corrupt("freshness report is invalid"))?;
    let prepared = PreparedExplanationFreshness {
        report,
        report_bytes: report_lease.bytes().to_vec(),
        report_ref: root.report_ref.clone(),
        projection: root.projection.clone(),
    };
    if canonical::bytes(&prepared.report).map_err(internal)? != prepared.report_bytes {
        return Err(corrupt("freshness report is not canonical"));
    }
    let (old_explanation_root, old_flow_root, old_flow, explanation) =
        explanation_service::load_verified(
            state,
            store,
            &old_thread,
            &prepared.report.request.old_explanation_id,
        )?;
    let (_old_callable_root, old_facts, verified_old_flow) =
        thread_flow_service::load_verified(state, store, &old_thread, &old_flow_root.flow_id)?;
    if verified_old_flow != old_flow
        || old_explanation_root.explanation_id != prepared.report.request.old_explanation_id
    {
        return Err(corrupt("freshness old binding chain changed"));
    }
    let (against_flow_root, against_facts, against_flow) = thread_flow_service::load_verified(
        state,
        store,
        against_thread,
        &prepared.report.request.against_flow_id,
    )?;
    if against_flow_root.fact_set_id != prepared.report.request.against_fact_set_id {
        return Err(corrupt("freshness against binding chain changed"));
    }
    crate::explanation_freshness::verify_prepared(
        FreshnessSide {
            thread_id: &old_thread.thread_id,
            thread_authority_digest: &old_thread.authority_digest,
            fact_set: &old_facts,
            flow: &old_flow,
        },
        &explanation,
        FreshnessSide {
            thread_id: &against_thread.thread_id,
            thread_authority_digest: &against_thread.authority_digest,
            fact_set: &against_facts,
            flow: &against_flow,
        },
        &prepared,
    )?;
    let _leases = verify_closure(store, &prepared, true)?;
    Ok((root, prepared))
}

pub fn bounded_stdout(root: &ExplanationFreshnessRoot) -> Result<Value, ClewError> {
    validate_root(root)?;
    let value = json!({
        "schema": EXPLANATION_FRESHNESS_RESULT_SCHEMA,
        "oldThreadId": root.old_thread_id,
        "againstThreadId": root.against_thread_id,
        "freshnessId": root.freshness_id,
        "reportRef": root.report_ref,
        "freshness": root.projection,
    });
    if canonical::bytes(&value)
        .map_err(internal)?
        .len()
        .saturating_add(1)
        > MAX_FRESHNESS_STDOUT_BYTES
    {
        return Err(budget("freshness stdout exceeds 64 KiB"));
    }
    Ok(value)
}

fn root_from_prepared(
    old_thread: &ThreadAuthority,
    against_thread: &ThreadAuthority,
    prepared: &PreparedExplanationFreshness,
) -> Result<ExplanationFreshnessRoot, ClewError> {
    let root = ExplanationFreshnessRoot {
        schema: EXPLANATION_FRESHNESS_ROOT_SCHEMA.into(),
        freshness_id: prepared.report.freshness_id.clone(),
        old_thread_id: old_thread.thread_id.clone(),
        old_thread_authority_digest: old_thread.authority_digest.clone(),
        against_thread_id: against_thread.thread_id.clone(),
        against_thread_authority_digest: against_thread.authority_digest.clone(),
        report_ref: prepared.report_ref.clone(),
        projection: prepared.projection.clone(),
    };
    validate_root(&root)?;
    Ok(root)
}

fn validate_root(root: &ExplanationFreshnessRoot) -> Result<(), ClewError> {
    validate_freshness_id(&root.freshness_id)?;
    if root.schema != EXPLANATION_FRESHNESS_ROOT_SCHEMA
        || root.report_ref.object_schema != EXPLANATION_FRESHNESS_REPORT_SCHEMA
        || root.projection.freshness_id != root.freshness_id
    {
        return Err(corrupt("freshness retained root authority is invalid"));
    }
    Ok(())
}

fn verify_closure(
    store: &CasStore,
    prepared: &PreparedExplanationFreshness,
    report_published: bool,
) -> Result<Vec<CasLease>, ClewError> {
    let mut leases = Vec::new();
    for object in &prepared.report.retained_closure {
        let size = usize::try_from(object.size)
            .map_err(|_| budget("freshness closure object exceeds host size"))?;
        leases.push(store.read(object, size)?);
    }
    if report_published {
        leases.push(store.read(&prepared.report_ref, prepared.report_bytes.len())?);
    }
    Ok(leases)
}

fn publish_prepared(
    state: &StateAuthority,
    store: &CasStore,
    against_thread: &ThreadAuthority,
    prepared: &PreparedExplanationFreshness,
    root: &ExplanationFreshnessRoot,
) -> Result<(), ClewError> {
    let path = freshness_root_path(state, against_thread, &root.freshness_id)?;
    let root_bytes = canonical::bytes(root).map_err(internal)?;
    if state.private_file_exists(&path)? {
        let existing = state.read_private_file(&path, MAX_FRESHNESS_ROOT_BYTES)?;
        if existing != root_bytes {
            return Err(ClewError::new(
                ErrorCode::BindingChanged,
                "freshness identifier is already bound to another report",
            ));
        }
        store.read(&prepared.report_ref, prepared.report_bytes.len())?;
        return Ok(());
    }
    let published = store.put(EXPLANATION_FRESHNESS_REPORT_SCHEMA, &prepared.report_bytes)?;
    if published != prepared.report_ref {
        return Err(corrupt("CAS published another freshness report identity"));
    }
    write_json_create_new(state, &path, root)
}

fn freshness_root_path(
    state: &StateAuthority,
    thread: &ThreadAuthority,
    freshness_id: &str,
) -> Result<std::path::PathBuf, ClewError> {
    let digest = validate_freshness_id(freshness_id)?;
    let directory = state
        .thread_root(&thread.thread_id)?
        .join("explanation-freshness");
    state.directory_at(&directory)?;
    Ok(directory.join(format!("{digest}.json")))
}

fn validate_freshness_id(value: &str) -> Result<&str, ClewError> {
    value
        .strip_prefix("explanation-freshness:sha256:")
        .filter(|digest| {
            digest.len() == 64
                && digest
                    .bytes()
                    .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        })
        .ok_or_else(|| invalid("freshness id is invalid"))
}

fn write_json_create_new<T: Serialize>(
    state: &StateAuthority,
    path: &Path,
    value: &T,
) -> Result<(), ClewError> {
    let relative = path
        .strip_prefix(state.root())
        .map_err(|_| invalid("freshness root escapes managed state"))?;
    let parent = relative
        .parent()
        .ok_or_else(|| invalid("freshness root has no parent"))?;
    let name = relative
        .file_name()
        .ok_or_else(|| invalid("freshness root has no file name"))?;
    let bytes = canonical::bytes(value).map_err(internal)?;
    let directory = state.directory(parent)?;
    if directory.atomic_create(name, &bytes)? {
        return Ok(());
    }
    if directory.read_file(name, MAX_FRESHNESS_ROOT_BYTES)? == bytes {
        Ok(())
    } else {
        Err(ClewError::new(
            ErrorCode::BindingChanged,
            "freshness root was concurrently bound to another report",
        ))
    }
}

fn invalid(message: impl Into<String>) -> ClewError {
    ClewError::new(ErrorCode::InvalidInput, message)
}

fn corrupt(message: impl Into<String>) -> ClewError {
    ClewError::new(ErrorCode::StateCorrupt, message)
}

fn budget(message: impl Into<String>) -> ClewError {
    ClewError::new(ErrorCode::SliceBudgetExceeded, message)
}

fn internal(error: impl std::fmt::Display) -> ClewError {
    ClewError::new(ErrorCode::Internal, error.to_string())
}
