//! Managed publication and retained loading for exact-root static flow slices.

use crate::canonical;
use crate::cas::{CasLease, CasObject, CasStore};
use crate::error::{ClewError, ErrorCode};
use crate::generation_v2::{GENERATION_SCHEMA, GenerationManifest};
use crate::state::StateAuthority;
use crate::thread::ThreadAuthority;
use crate::thread_callables::{PreparedCallableFactSet, SourceAnchor};
use crate::thread_callables_service::{
    self, ThreadCallableRoot, load_verified as load_callable_verified,
};
use crate::thread_flow::{
    FLOW_SLICE_SCHEMA, FlowBudgets, FlowDirection, FlowRequest, FlowRootKind, FlowSlice,
    FlowSliceProjection, MAX_FLOW_SLICE_BYTES, MAX_FLOW_STDOUT_BYTES, PreparedFlowSlice,
};
use crate::thread_flow_cfg::{
    LOCAL_CFG_PAYLOAD_SCHEMA, LocalCfgCatalog, LocalCfgPayload, LocalCfgSupport, PreparedLocalCfg,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::path::{Component, Path};

pub const THREAD_FLOW_ROOT_SCHEMA: &str = "codeclew-thread-flow-root/0.1";
pub const THREAD_FLOW_RESULT_SCHEMA: &str = "codeclew-thread-flow-result/0.1";
pub const MAX_FLOW_RETAINED_CLOSURE_BYTES: usize = 64 * 1024 * 1024;
const MAX_THREAD_FLOW_ROOT_BYTES: usize = 256 * 1024;

#[derive(Debug, Clone)]
pub struct ThreadFlowServiceRequest {
    pub pair_id: String,
    pub member_alias: String,
    pub root_kind: FlowRootKind,
    pub root: String,
    pub direction: FlowDirection,
    pub max_depth: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ThreadFlowRoot {
    pub schema: String,
    pub flow_id: String,
    pub thread_id: String,
    pub thread_authority_digest: String,
    pub fact_set_id: String,
    pub fact_set_authority_digest: String,
    pub slice_ref: CasObject,
    pub projection: FlowSliceProjection,
}

pub fn create(
    thread: &ThreadAuthority,
    fact_set_id: &str,
    request: ThreadFlowServiceRequest,
) -> Result<ThreadFlowRoot, ClewError> {
    let state = StateAuthority::process_default()?;
    create_with_state(&state, thread, fact_set_id, request)
}

pub(crate) fn create_with_state(
    state: &StateAuthority,
    thread: &ThreadAuthority,
    fact_set_id: &str,
    request: ThreadFlowServiceRequest,
) -> Result<ThreadFlowRoot, ClewError> {
    thread.verify()?;
    thread.require_open_with_state(state)?;
    let store = CasStore::open(state)?;
    let (callable_root, fact_set) = load_callable_verified(state, &store, thread, fact_set_id)?;
    let cfg = load_cfg_catalog(&store, &fact_set, &request.member_alias)?;
    let prepared = crate::thread_flow::build_with_cfg(
        FlowRequest {
            thread_id: thread.thread_id.clone(),
            thread_authority_digest: thread.authority_digest.clone(),
            fact_set_id: callable_root.fact_set_id.clone(),
            fact_set_authority_digest: callable_root.authority.authority_digest.clone(),
            pair_id: request.pair_id,
            member_alias: request.member_alias,
            root_kind: request.root_kind,
            root: request.root,
            direction: request.direction,
            budgets: FlowBudgets::frozen(request.max_depth)?,
        },
        &fact_set,
        &cfg,
    )?;
    crate::thread_flow::verify_prepared_with_cfg(&prepared, &fact_set, &cfg)?;
    let root = root_from_prepared(thread, &callable_root, &prepared)?;
    bounded_stdout(&root)?;
    let _leases = verify_support_closure(&store, &fact_set, &prepared, false)?;

    let _admission = thread.admit_with_state(state)?;
    crate::thread::revalidate_authority_record(state, thread)?;
    thread_callables_service::revalidate_root_record(state, thread, &callable_root)?;
    publish_prepared(state, &store, thread, &prepared, &root)?;
    Ok(root)
}

pub fn load(thread: &ThreadAuthority, flow_id: &str) -> Result<ThreadFlowRoot, ClewError> {
    let state = StateAuthority::process_default()?;
    let store = CasStore::open(&state)?;
    load_verified(&state, &store, thread, flow_id).map(|(root, _, _)| root)
}

pub(crate) fn load_verified(
    state: &StateAuthority,
    store: &CasStore,
    thread: &ThreadAuthority,
    flow_id: &str,
) -> Result<(ThreadFlowRoot, PreparedCallableFactSet, PreparedFlowSlice), ClewError> {
    thread.verify()?;
    let path = flow_root_path(state, thread, flow_id)?;
    let bytes = state
        .read_private_file(&path, MAX_THREAD_FLOW_ROOT_BYTES)
        .map_err(|_| invalid("thread flow root is missing or exceeds 256 KiB"))?;
    let root: ThreadFlowRoot =
        serde_json::from_slice(&bytes).map_err(|_| corrupt("thread flow root is invalid"))?;
    if canonical::bytes(&root).map_err(internal)? != bytes
        || root.flow_id != flow_id
        || root.thread_id != thread.thread_id
        || root.thread_authority_digest != thread.authority_digest
    {
        return Err(corrupt("thread flow root authority is invalid"));
    }
    validate_root(&root)?;
    let (callable_root, fact_set) =
        load_callable_verified(state, store, thread, &root.fact_set_id)?;
    if callable_root.authority.authority_digest != root.fact_set_authority_digest {
        return Err(corrupt("thread flow parent fact-set authority changed"));
    }
    let size = usize::try_from(root.slice_ref.size)
        .map_err(|_| budget("thread flow slice exceeds host size"))?;
    if size > MAX_FLOW_SLICE_BYTES {
        return Err(budget("thread flow slice exceeds 64 MiB"));
    }
    let lease = store.read(&root.slice_ref, size)?;
    let slice_bytes = lease.bytes().to_vec();
    let slice: FlowSlice = serde_json::from_slice(&slice_bytes)
        .map_err(|_| corrupt("thread flow slice is invalid"))?;
    if canonical::bytes(&slice).map_err(internal)? != slice_bytes {
        return Err(corrupt("thread flow slice is not canonical"));
    }
    let prepared = PreparedFlowSlice {
        slice,
        slice_bytes,
        slice_ref: root.slice_ref.clone(),
        projection: root.projection.clone(),
    };
    let cfg = load_cfg_catalog(store, &fact_set, &prepared.slice.request.member_alias)?;
    let is_t00_slice = prepared.slice.control_flow_regions.is_empty()
        && !prepared
            .slice
            .boundaries
            .iter()
            .any(|boundary| boundary.code == "VERIFY_CONTROL_FLOW_ORDER");
    if is_t00_slice {
        crate::thread_flow::verify_prepared(&prepared, &fact_set)?;
    } else {
        crate::thread_flow::verify_prepared_with_cfg(&prepared, &fact_set, &cfg)?;
    }
    if prepared.slice.request.thread_id != root.thread_id
        || prepared.slice.request.thread_authority_digest != root.thread_authority_digest
        || prepared.slice.request.fact_set_id != root.fact_set_id
        || prepared.slice.request.fact_set_authority_digest != root.fact_set_authority_digest
    {
        return Err(corrupt(
            "thread flow slice is not bound to its retained root",
        ));
    }
    let _leases = verify_support_closure(store, &fact_set, &prepared, true)?;
    Ok((root, fact_set, prepared))
}

pub fn bounded_stdout(root: &ThreadFlowRoot) -> Result<Value, ClewError> {
    validate_root(root)?;
    let value = json!({
        "schema": THREAD_FLOW_RESULT_SCHEMA,
        "threadId": root.thread_id,
        "threadAuthorityDigest": root.thread_authority_digest,
        "factSetId": root.fact_set_id,
        "factSetAuthorityDigest": root.fact_set_authority_digest,
        "flowId": root.flow_id,
        "sliceRef": root.slice_ref,
        "flow": root.projection,
    });
    if canonical::bytes(&value)
        .map_err(internal)?
        .len()
        .saturating_add(1)
        > MAX_FLOW_STDOUT_BYTES
    {
        return Err(budget("thread flow stdout exceeds 64 KiB"));
    }
    Ok(value)
}

fn root_from_prepared(
    thread: &ThreadAuthority,
    callable_root: &ThreadCallableRoot,
    prepared: &PreparedFlowSlice,
) -> Result<ThreadFlowRoot, ClewError> {
    let root = ThreadFlowRoot {
        schema: THREAD_FLOW_ROOT_SCHEMA.into(),
        flow_id: prepared.slice.flow_id.clone(),
        thread_id: thread.thread_id.clone(),
        thread_authority_digest: thread.authority_digest.clone(),
        fact_set_id: callable_root.fact_set_id.clone(),
        fact_set_authority_digest: callable_root.authority.authority_digest.clone(),
        slice_ref: prepared.slice_ref.clone(),
        projection: prepared.projection.clone(),
    };
    validate_root(&root)?;
    Ok(root)
}

fn validate_root(root: &ThreadFlowRoot) -> Result<(), ClewError> {
    validate_flow_id(&root.flow_id)?;
    if root.schema != THREAD_FLOW_ROOT_SCHEMA
        || root.slice_ref.object_schema != FLOW_SLICE_SCHEMA
        || root.projection.flow_id != root.flow_id
    {
        return Err(corrupt("thread flow retained root authority is invalid"));
    }
    Ok(())
}

fn publish_prepared(
    state: &StateAuthority,
    store: &CasStore,
    thread: &ThreadAuthority,
    prepared: &PreparedFlowSlice,
    root: &ThreadFlowRoot,
) -> Result<(), ClewError> {
    let path = flow_root_path(state, thread, &root.flow_id)?;
    let root_bytes = canonical::bytes(root).map_err(internal)?;
    if state.private_file_exists(&path)? {
        let existing = state.read_private_file(&path, MAX_THREAD_FLOW_ROOT_BYTES)?;
        if existing != root_bytes {
            return Err(ClewError::new(
                ErrorCode::BindingChanged,
                "flow identifier is already bound to another slice",
            ));
        }
        store.read(&prepared.slice_ref, prepared.slice_bytes.len())?;
        return Ok(());
    }
    let published = store.put(FLOW_SLICE_SCHEMA, &prepared.slice_bytes)?;
    if published != prepared.slice_ref {
        return Err(corrupt(
            "CAS publication returned another flow slice identity",
        ));
    }
    write_json_create_new(state, &path, root)
}

fn verify_support_closure(
    store: &CasStore,
    fact_set: &PreparedCallableFactSet,
    prepared: &PreparedFlowSlice,
    slice_published: bool,
) -> Result<Vec<CasLease>, ClewError> {
    let parent = fact_set
        .authority
        .direct_cas_closure
        .iter()
        .map(|reference| (reference.digest.as_str(), reference))
        .collect::<BTreeMap<_, _>>();
    let fact_shards = fact_set
        .authority
        .fact_shards
        .iter()
        .map(|reference| (reference.object.digest.as_str(), &reference.object))
        .collect::<BTreeMap<_, _>>();
    let mut sources = BTreeMap::<String, (CasObject, Vec<SourceAnchor>)>::new();
    let supports = prepared
        .slice
        .nodes
        .iter()
        .flat_map(|node| &node.support_refs)
        .chain(
            prepared
                .slice
                .edges
                .iter()
                .flat_map(|edge| &edge.support_refs),
        )
        .chain(
            prepared
                .slice
                .boundaries
                .iter()
                .flat_map(|boundary| &boundary.support_refs),
        );
    for support in supports {
        if fact_shards.get(support.fact_shard_ref.digest.as_str()) != Some(&&support.fact_shard_ref)
            || parent.get(support.input_payload_ref.digest.as_str())
                != Some(&&support.input_payload_ref)
            || parent.get(support.provenance.generation_ref.digest.as_str())
                != Some(&&support.provenance.generation_ref)
            || support.provenance.input_payload_ref != support.input_payload_ref
        {
            return Err(corrupt("thread flow support escapes its parent fact set"));
        }
        if let Some(anchor) = &support.source {
            validate_relative_source(&anchor.path)?;
            if support.provenance.source.as_ref() != Some(anchor)
                || parent.get(anchor.content_ref.digest.as_str()) != Some(&&anchor.content_ref)
            {
                return Err(corrupt("thread flow source escapes its parent fact set"));
            }
            match sources.entry(anchor.content_ref.digest.clone()) {
                std::collections::btree_map::Entry::Vacant(entry) => {
                    entry.insert((anchor.content_ref.clone(), vec![anchor.clone()]));
                }
                std::collections::btree_map::Entry::Occupied(mut entry) => {
                    if entry.get().0 != anchor.content_ref {
                        return Err(corrupt("flow source digest has conflicting authority"));
                    }
                    entry.get_mut().1.push(anchor.clone());
                }
            }
        }
    }
    let mut cfg_payloads = BTreeMap::<String, CasObject>::new();
    for region in &prepared.slice.control_flow_regions {
        if parent.get(region.support.generation_ref.digest.as_str())
            != Some(&&region.support.generation_ref)
        {
            return Err(corrupt("local CFG generation escapes its parent fact set"));
        }
        let bytes = canonical::bytes(&region.graph).map_err(internal)?;
        if CasObject::for_bytes(LOCAL_CFG_PAYLOAD_SCHEMA, &bytes)? != region.support.payload_ref {
            return Err(corrupt(
                "local CFG graph differs from its payload authority",
            ));
        }
        match cfg_payloads.entry(region.support.payload_ref.digest.clone()) {
            std::collections::btree_map::Entry::Vacant(entry) => {
                entry.insert(region.support.payload_ref.clone());
            }
            std::collections::btree_map::Entry::Occupied(entry) => {
                if entry.get() != &region.support.payload_ref {
                    return Err(corrupt(
                        "local CFG payload digest has conflicting authority",
                    ));
                }
            }
        }
    }
    let cfg_bytes = cfg_payloads.values().try_fold(0usize, |total, reference| {
        let size = usize::try_from(reference.size)
            .map_err(|_| budget("local CFG payload exceeds host size"))?;
        store.read(reference, size)?;
        total
            .checked_add(size)
            .ok_or_else(|| budget("local CFG retained bytes overflowed"))
    })?;
    let inherited_bytes =
        fact_set
            .authority
            .direct_cas_closure
            .iter()
            .try_fold(0usize, |total, reference| {
                let size = usize::try_from(reference.size)
                    .map_err(|_| budget("flow parent object exceeds host size"))?;
                total
                    .checked_add(size)
                    .ok_or_else(|| budget("flow retained byte count overflowed"))
            })?;
    if inherited_bytes
        .saturating_add(cfg_bytes)
        .saturating_add(prepared.slice_bytes.len())
        > MAX_FLOW_RETAINED_CLOSURE_BYTES
    {
        return Err(budget("thread flow retained closure exceeds 64 MiB"));
    }
    if slice_published {
        store.read(&prepared.slice_ref, prepared.slice_bytes.len())?;
    }
    let mut leases = Vec::with_capacity(sources.len());
    for (_digest, (reference, anchors)) in sources {
        let size =
            usize::try_from(reference.size).map_err(|_| budget("flow source exceeds host size"))?;
        let lease = store.read(&reference, size)?;
        let text = std::str::from_utf8(lease.bytes())
            .map_err(|_| corrupt("flow source evidence is not UTF-8"))?;
        for anchor in anchors {
            match (anchor.start, anchor.end) {
                (Some(start), Some(end)) => {
                    let start = usize::try_from(start)
                        .map_err(|_| corrupt("flow source start exceeds host size"))?;
                    let end = usize::try_from(end)
                        .map_err(|_| corrupt("flow source end exceeds host size"))?;
                    if start > end
                        || end > text.len()
                        || !text.is_char_boundary(start)
                        || !text.is_char_boundary(end)
                    {
                        return Err(corrupt("flow source anchor is outside its CAS object"));
                    }
                }
                (None, None) => {}
                _ => return Err(corrupt("flow source anchor has one range endpoint")),
            }
        }
        leases.push(lease);
    }
    Ok(leases)
}

fn load_cfg_catalog(
    store: &CasStore,
    fact_set: &PreparedCallableFactSet,
    member_alias: &str,
) -> Result<LocalCfgCatalog, ClewError> {
    let member = fact_set
        .authority
        .members
        .iter()
        .find(|member| member.member_alias == member_alias)
        .ok_or_else(|| invalid("flow member has no callable authority"))?;
    let mut catalog = LocalCfgCatalog::default();
    for compilation in &member.compilations {
        let generation_size = usize::try_from(compilation.generation_ref.size)
            .map_err(|_| budget("flow generation manifest exceeds host size"))?;
        let generation_lease = store.read(&compilation.generation_ref, generation_size)?;
        let generation: GenerationManifest = serde_json::from_slice(generation_lease.bytes())
            .map_err(|_| corrupt("flow generation manifest is invalid"))?;
        if compilation.generation_ref.object_schema != GENERATION_SCHEMA
            || canonical::bytes(&generation).map_err(internal)? != generation_lease.bytes()
        {
            return Err(corrupt("flow generation manifest is not canonical"));
        }
        generation.visit_facts(store, |fact| {
            if !fact.fact_key.starts_with("kotlin:local-cfg:") {
                return Ok(());
            }
            if fact.payload.object_schema != LOCAL_CFG_PAYLOAD_SCHEMA {
                return Err(corrupt("local CFG fact has another payload schema"));
            }
            let payload_size = usize::try_from(fact.payload.size)
                .map_err(|_| budget("local CFG payload exceeds host size"))?;
            if payload_size > 4 * 1024 * 1024 {
                return Err(budget("local CFG payload exceeds 4 MiB"));
            }
            let payload_lease = store.read(&fact.payload, payload_size)?;
            let payload: LocalCfgPayload = serde_json::from_slice(payload_lease.bytes())
                .map_err(|_| corrupt("local CFG payload is invalid"))?;
            if canonical::bytes(&payload).map_err(internal)? != payload_lease.bytes() {
                return Err(corrupt("local CFG payload is not canonical"));
            }
            catalog.insert(PreparedLocalCfg {
                payload,
                support: LocalCfgSupport {
                    member_alias: member_alias.into(),
                    compilation_id: compilation.compilation_id.clone(),
                    generation_ref: compilation.generation_ref.clone(),
                    payload_ref: fact.payload.clone(),
                },
            })
        })?;
    }
    Ok(catalog)
}

fn flow_root_path(
    state: &StateAuthority,
    thread: &ThreadAuthority,
    flow_id: &str,
) -> Result<std::path::PathBuf, ClewError> {
    let digest = validate_flow_id(flow_id)?;
    let directory = state.thread_root(&thread.thread_id)?.join("flows");
    state.directory_at(&directory)?;
    Ok(directory.join(format!("{digest}.json")))
}

fn validate_flow_id(flow_id: &str) -> Result<&str, ClewError> {
    flow_id
        .strip_prefix("thread-flow:sha256:")
        .filter(|digest| {
            digest.len() == 64
                && digest
                    .bytes()
                    .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        })
        .ok_or_else(|| invalid("thread flow id is invalid"))
}

fn write_json_create_new<T: Serialize>(
    state: &StateAuthority,
    path: &Path,
    value: &T,
) -> Result<(), ClewError> {
    let relative = path
        .strip_prefix(state.root())
        .map_err(|_| invalid("thread flow root escapes managed state"))?;
    let parent = relative
        .parent()
        .ok_or_else(|| invalid("thread flow root has no parent"))?;
    let name = relative
        .file_name()
        .ok_or_else(|| invalid("thread flow root has no file name"))?;
    let bytes = canonical::bytes(value).map_err(internal)?;
    let directory = state.directory(parent)?;
    if directory.atomic_create(name, &bytes)? {
        return Ok(());
    }
    if directory.read_file(name, MAX_THREAD_FLOW_ROOT_BYTES)? == bytes {
        Ok(())
    } else {
        Err(ClewError::new(
            ErrorCode::BindingChanged,
            "thread flow root was concurrently bound to another slice",
        ))
    }
}

fn validate_relative_source(path: &str) -> Result<(), ClewError> {
    let path = Path::new(path);
    if path.as_os_str().is_empty()
        || path.is_absolute()
        || path.components().any(|component| {
            !matches!(component, Component::Normal(_)) || component.as_os_str().to_str().is_none()
        })
    {
        return Err(corrupt("flow source path is not repository-relative"));
    }
    Ok(())
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
