//! Managed publication and retained loading for Kotlin thread impacts.

use crate::canonical;
use crate::cas::{CasLease, CasObject, CasStore};
use crate::error::{ClewError, ErrorCode};
use crate::state::StateAuthority;
use crate::thread::ThreadAuthority;
use crate::thread_callables::PreparedCallableFactSet;
use crate::thread_callables_service::{
    self, ThreadCallableRoot, load_verified as load_callable_verified,
};
use crate::thread_impact::{
    ImpactBudgets, KotlinImpactSubject, MAX_IMPACT_RETAINED_CLOSURE_BYTES, MAX_IMPACT_STDOUT_BYTES,
    PreparedThreadImpact, THREAD_IMPACT_EVIDENCE_SCHEMA, ThreadImpactAuthority,
    ThreadImpactEvidence, ThreadImpactProjection, ThreadImpactRequest,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::path::{Component, Path};

#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CreateTestHookPoint {
    AfterInitialOpen,
    AfterAdmission,
}

#[cfg(test)]
type CreateTestHook = Box<dyn Fn(CreateTestHookPoint)>;

#[cfg(test)]
std::thread_local! {
    static CREATE_TEST_HOOK: std::cell::RefCell<Option<CreateTestHook>> =
        std::cell::RefCell::new(None);
}

#[cfg(test)]
fn invoke_create_test_hook(point: CreateTestHookPoint) {
    CREATE_TEST_HOOK.with(|slot| {
        if let Some(hook) = slot.borrow().as_ref() {
            hook(point);
        }
    });
}

#[cfg(test)]
fn with_create_test_hook<T>(
    hook: impl Fn(CreateTestHookPoint) + 'static,
    action: impl FnOnce() -> T,
) -> T {
    CREATE_TEST_HOOK.with(|slot| {
        assert!(slot.borrow_mut().replace(Box::new(hook)).is_none());
    });
    let result = action();
    CREATE_TEST_HOOK.with(|slot| {
        slot.borrow_mut().take();
    });
    result
}

pub const THREAD_IMPACT_ROOT_SCHEMA: &str = "codeclew-thread-impact-root/1.0";
pub const THREAD_IMPACT_RESULT_SCHEMA: &str = "codeclew-thread-impact-result/1.0";
const MAX_THREAD_IMPACT_ROOT_BYTES: usize = 65 * 1024 * 1024;

#[derive(Debug, Clone)]
pub struct ThreadImpactServiceRequest {
    pub pair_id: String,
    pub subject: KotlinImpactSubject,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ThreadImpactRoot {
    pub schema: String,
    pub impact_id: String,
    pub thread_id: String,
    pub thread_authority_digest: String,
    pub fact_set_id: String,
    pub fact_set_authority_digest: String,
    pub authority: ThreadImpactAuthority,
    pub projection: ThreadImpactProjection,
}

pub(crate) struct VerifiedImpactBundle {
    pub impact_root: ThreadImpactRoot,
    pub callable_root: ThreadCallableRoot,
    pub fact_set: PreparedCallableFactSet,
    pub impact: PreparedThreadImpact,
}

pub fn create(
    thread: &ThreadAuthority,
    fact_set_id: &str,
    request: ThreadImpactServiceRequest,
) -> Result<ThreadImpactRoot, ClewError> {
    let state = StateAuthority::process_default()?;
    create_with_state(&state, thread, fact_set_id, request)
}

pub(crate) fn create_with_state(
    state: &StateAuthority,
    thread: &ThreadAuthority,
    fact_set_id: &str,
    request: ThreadImpactServiceRequest,
) -> Result<ThreadImpactRoot, ClewError> {
    thread.verify()?;
    thread.require_open_with_state(state)?;
    #[cfg(test)]
    invoke_create_test_hook(CreateTestHookPoint::AfterInitialOpen);
    let store = CasStore::open(state)?;
    let (callable_root, fact_set) = load_callable_verified(state, &store, thread, fact_set_id)?;
    let prepared = crate::thread_impact::build_from_verified(
        &fact_set,
        ThreadImpactRequest {
            fact_set_authority_digest: callable_root.authority.authority_digest.clone(),
            pair_id: request.pair_id,
            subject: request.subject,
            budgets: ImpactBudgets::frozen(),
        },
    )?;
    crate::thread_impact::verify_prepared_from_verified(&fact_set, &prepared)?;
    let root = root_from_prepared(thread, &callable_root, &prepared)?;
    bounded_stdout(&root)?;
    verify_inherited_closure(&callable_root, &prepared)?;
    let _source_leases = validate_selected_sources(&store, &prepared.evidence)?;
    verify_closure_objects(&store, &prepared, false)?;

    // Query and source validation happen outside the admission. Publication is
    // the only linearized section, so a concurrent close either wins with no
    // retained root or waits for one complete root.
    let _admission = thread.admit_with_state(state)?;
    #[cfg(test)]
    invoke_create_test_hook(CreateTestHookPoint::AfterAdmission);
    crate::thread::revalidate_authority_record(state, thread)?;
    thread_callables_service::revalidate_root_record(state, thread, &callable_root)?;
    publish_prepared(state, &store, thread, &prepared, &root)?;
    Ok(root)
}

pub fn load(thread: &ThreadAuthority, impact_id: &str) -> Result<ThreadImpactRoot, ClewError> {
    let state = StateAuthority::process_default()?;
    let store = CasStore::open(&state)?;
    load_verified(&state, &store, thread, impact_id).map(|(root, _callables, _impact)| root)
}

pub(crate) fn load_verified(
    state: &StateAuthority,
    store: &CasStore,
    thread: &ThreadAuthority,
    impact_id: &str,
) -> Result<
    (
        ThreadImpactRoot,
        PreparedCallableFactSet,
        PreparedThreadImpact,
    ),
    ClewError,
> {
    let bundle = load_verified_bundle(state, store, thread, impact_id)?;
    Ok((bundle.impact_root, bundle.fact_set, bundle.impact))
}

pub(crate) fn load_verified_bundle(
    state: &StateAuthority,
    store: &CasStore,
    thread: &ThreadAuthority,
    impact_id: &str,
) -> Result<VerifiedImpactBundle, ClewError> {
    thread.verify()?;
    let path = impact_root_path(state, thread, impact_id)?;
    let bytes = state
        .read_private_file(&path, MAX_THREAD_IMPACT_ROOT_BYTES)
        .map_err(|_| invalid("thread impact root is missing or exceeds 65 MiB"))?;
    let root: ThreadImpactRoot =
        serde_json::from_slice(&bytes).map_err(|_| corrupt("thread impact root is invalid"))?;
    if canonical::bytes(&root).map_err(internal)? != bytes
        || root.impact_id != impact_id
        || root.thread_id != thread.thread_id
        || root.thread_authority_digest != thread.authority_digest
    {
        return Err(corrupt("thread impact root authority is invalid"));
    }
    validate_root(&root)?;
    let (callable_root, fact_set) =
        load_callable_verified(state, store, thread, &root.fact_set_id)?;
    if callable_root.authority.authority_digest != root.fact_set_authority_digest {
        return Err(corrupt("thread impact parent fact-set authority changed"));
    }
    let evidence_object = read_prepared_object(store, &root.authority.evidence_ref)?;
    let evidence: ThreadImpactEvidence = serde_json::from_slice(&evidence_object.bytes)
        .map_err(|_| corrupt("thread impact evidence is invalid"))?;
    if canonical::bytes(&evidence).map_err(internal)? != evidence_object.bytes {
        return Err(corrupt("thread impact evidence is not canonical"));
    }
    let prepared = PreparedThreadImpact {
        authority: root.authority.clone(),
        evidence,
        evidence_object,
        authority_bytes: canonical::bytes(&root.authority).map_err(internal)?,
        projection: root.projection.clone(),
        projection_bytes: canonical::bytes(&root.projection).map_err(internal)?,
    };
    crate::thread_impact::verify_prepared_from_verified(&fact_set, &prepared)?;
    verify_inherited_closure(&callable_root, &prepared)?;
    let _source_leases = validate_selected_sources(store, &prepared.evidence)?;
    verify_closure_objects(store, &prepared, true)?;
    Ok(VerifiedImpactBundle {
        impact_root: root,
        callable_root,
        fact_set,
        impact: prepared,
    })
}

pub(crate) fn revalidate_root_record(
    state: &StateAuthority,
    thread: &ThreadAuthority,
    expected: &ThreadImpactRoot,
) -> Result<(), ClewError> {
    let root_path = impact_root_path(state, thread, &expected.impact_id)?;
    let bytes = state
        .read_private_file(&root_path, MAX_THREAD_IMPACT_ROOT_BYTES)
        .map_err(|_| {
            ClewError::new(
                ErrorCode::BindingChanged,
                "thread impact root disappeared before derived publication",
            )
        })?;
    if bytes != canonical::bytes(expected).map_err(internal)? {
        return Err(ClewError::new(
            ErrorCode::BindingChanged,
            "thread impact root changed before derived publication",
        ));
    }
    Ok(())
}

pub fn bounded_stdout(root: &ThreadImpactRoot) -> Result<Value, ClewError> {
    validate_root(root)?;
    let value = json!({
        "schema": THREAD_IMPACT_RESULT_SCHEMA,
        "threadId": root.thread_id,
        "threadAuthorityDigest": root.thread_authority_digest,
        "factSetId": root.fact_set_id,
        "factSetAuthorityDigest": root.fact_set_authority_digest,
        "impactId": root.impact_id,
        "authorityDigest": root.authority.authority_digest,
        "evidenceRef": root.authority.evidence_ref,
        "impact": root.projection,
    });
    if canonical::bytes(&value)
        .map_err(internal)?
        .len()
        .saturating_add(1)
        > MAX_IMPACT_STDOUT_BYTES
    {
        return Err(budget("thread impact stdout exceeds 64 KiB"));
    }
    Ok(value)
}

fn root_from_prepared(
    thread: &ThreadAuthority,
    callable_root: &ThreadCallableRoot,
    prepared: &PreparedThreadImpact,
) -> Result<ThreadImpactRoot, ClewError> {
    let root = ThreadImpactRoot {
        schema: THREAD_IMPACT_ROOT_SCHEMA.into(),
        impact_id: prepared.projection.impact_id.clone(),
        thread_id: thread.thread_id.clone(),
        thread_authority_digest: thread.authority_digest.clone(),
        fact_set_id: callable_root.fact_set_id.clone(),
        fact_set_authority_digest: callable_root.authority.authority_digest.clone(),
        authority: prepared.authority.clone(),
        projection: prepared.projection.clone(),
    };
    validate_root(&root)?;
    Ok(root)
}

fn validate_root(root: &ThreadImpactRoot) -> Result<(), ClewError> {
    if root.schema != THREAD_IMPACT_ROOT_SCHEMA
        || root.impact_id != format!("thread-impact:{}", root.authority.authority_digest)
        || root.fact_set_authority_digest != root.authority.fact_set_authority_digest
        || root.projection != crate::thread_impact::project(&root.authority)
        || root.projection.impact_id != root.impact_id
        || root.projection.fact_set_authority_digest != root.fact_set_authority_digest
    {
        return Err(corrupt("thread impact retained root authority is invalid"));
    }
    Ok(())
}

fn publish_prepared(
    state: &StateAuthority,
    store: &CasStore,
    thread: &ThreadAuthority,
    prepared: &PreparedThreadImpact,
    root: &ThreadImpactRoot,
) -> Result<(), ClewError> {
    let path = impact_root_path(state, thread, &root.impact_id)?;
    let root_bytes = canonical::bytes(root).map_err(internal)?;
    if state.private_file_exists(&path)? {
        let existing = state.read_private_file(&path, MAX_THREAD_IMPACT_ROOT_BYTES)?;
        if existing != root_bytes {
            return Err(ClewError::new(
                ErrorCode::BindingChanged,
                "impact identifier is already bound to different evidence",
            ));
        }
        verify_closure_objects(store, prepared, true)?;
        return Ok(());
    }
    let published = store.put(
        THREAD_IMPACT_EVIDENCE_SCHEMA,
        &prepared.evidence_object.bytes,
    )?;
    if published != prepared.evidence_object.reference {
        return Err(corrupt(
            "CAS publication returned another thread impact evidence identity",
        ));
    }
    verify_closure_objects(store, prepared, true)?;
    write_json_create_new(state, &path, root)
}

fn verify_inherited_closure(
    callable_root: &ThreadCallableRoot,
    prepared: &PreparedThreadImpact,
) -> Result<(), ClewError> {
    let parent = callable_root
        .authority
        .direct_cas_closure
        .iter()
        .map(|reference| (reference.digest.as_str(), reference))
        .collect::<BTreeMap<_, _>>();
    let mut derived_count = 0usize;
    for reference in &prepared.authority.direct_cas_closure {
        if reference == &prepared.evidence_object.reference {
            derived_count += 1;
            continue;
        }
        if parent.get(reference.digest.as_str()) != Some(&reference) {
            return Err(corrupt(
                "thread impact closure contains evidence outside its parent fact set",
            ));
        }
    }
    if derived_count != prepared.authority.new_derived_cas_object_count
        || derived_count != 1
        || !prepared
            .authority
            .direct_cas_closure
            .iter()
            .any(|reference| reference == &prepared.authority.evidence_ref)
    {
        return Err(corrupt(
            "thread impact derived evidence closure is inconsistent",
        ));
    }
    Ok(())
}

fn verify_closure_objects(
    store: &CasStore,
    prepared: &PreparedThreadImpact,
    evidence_published: bool,
) -> Result<(), ClewError> {
    let mut total = 0usize;
    for reference in &prepared.authority.direct_cas_closure {
        let size = usize::try_from(reference.size)
            .map_err(|_| budget("impact retained object exceeds host size"))?;
        total = total
            .checked_add(size)
            .ok_or_else(|| budget("impact retained byte count overflowed"))?;
        if reference == &prepared.evidence_object.reference && !evidence_published {
            continue;
        }
        store.read(reference, size)?;
    }
    if total != prepared.authority.retained_cas_bytes || total > MAX_IMPACT_RETAINED_CLOSURE_BYTES {
        return Err(corrupt(
            "thread impact retained byte authority is inconsistent",
        ));
    }
    Ok(())
}

fn validate_selected_sources(
    store: &CasStore,
    evidence: &ThreadImpactEvidence,
) -> Result<Vec<CasLease>, ClewError> {
    let mut grouped =
        BTreeMap::<String, (CasObject, Vec<&crate::thread_callables::SourceAnchor>)>::new();
    for finding in &evidence.selection.findings {
        let Some(anchor) = &finding.evidence.provenance.source else {
            continue;
        };
        validate_relative_source(&anchor.path)?;
        match grouped.entry(anchor.content_ref.digest.clone()) {
            std::collections::btree_map::Entry::Vacant(entry) => {
                entry.insert((anchor.content_ref.clone(), vec![anchor]));
            }
            std::collections::btree_map::Entry::Occupied(mut entry) => {
                if entry.get().0 != anchor.content_ref {
                    return Err(corrupt(
                        "impact source digest repeats with conflicting authority",
                    ));
                }
                entry.get_mut().1.push(anchor);
            }
        }
    }
    for pointer in &evidence.selection.obligation_evidence {
        let Some(anchor) = &pointer.provenance.source else {
            continue;
        };
        validate_relative_source(&anchor.path)?;
        match grouped.entry(anchor.content_ref.digest.clone()) {
            std::collections::btree_map::Entry::Vacant(entry) => {
                entry.insert((anchor.content_ref.clone(), vec![anchor]));
            }
            std::collections::btree_map::Entry::Occupied(mut entry) => {
                if entry.get().0 != anchor.content_ref {
                    return Err(corrupt(
                        "impact source digest repeats with conflicting authority",
                    ));
                }
                entry.get_mut().1.push(anchor);
            }
        }
    }
    let mut leases = Vec::with_capacity(grouped.len());
    for (_digest, (reference, anchors)) in grouped {
        let size = usize::try_from(reference.size)
            .map_err(|_| budget("impact source exceeds host size"))?;
        let lease = store.read(&reference, size)?;
        let text = std::str::from_utf8(lease.bytes())
            .map_err(|_| corrupt("impact source evidence is not UTF-8"))?;
        for anchor in anchors {
            match (anchor.start, anchor.end) {
                (Some(start), Some(end)) => {
                    let start = usize::try_from(start)
                        .map_err(|_| corrupt("impact source start exceeds host size"))?;
                    let end = usize::try_from(end)
                        .map_err(|_| corrupt("impact source end exceeds host size"))?;
                    if start > end
                        || end > text.len()
                        || !text.is_char_boundary(start)
                        || !text.is_char_boundary(end)
                    {
                        return Err(corrupt(
                            "impact source anchor is outside its exact CAS object",
                        ));
                    }
                }
                (None, None) => {}
                _ => return Err(corrupt("impact source anchor has one range endpoint")),
            }
        }
        leases.push(lease);
    }
    Ok(leases)
}

fn read_prepared_object(
    store: &CasStore,
    reference: &CasObject,
) -> Result<crate::thread_callables::PreparedCasObject, ClewError> {
    if reference.object_schema != THREAD_IMPACT_EVIDENCE_SCHEMA {
        return Err(corrupt(
            "thread impact evidence reference has another schema",
        ));
    }
    let size = usize::try_from(reference.size)
        .map_err(|_| budget("thread impact evidence exceeds host size"))?;
    if size > MAX_IMPACT_RETAINED_CLOSURE_BYTES {
        return Err(budget("thread impact evidence exceeds 64 MiB"));
    }
    let lease = store.read(reference, size)?;
    Ok(crate::thread_callables::PreparedCasObject {
        reference: reference.clone(),
        bytes: lease.bytes().to_vec(),
    })
}

fn impact_root_path(
    state: &StateAuthority,
    thread: &ThreadAuthority,
    impact_id: &str,
) -> Result<std::path::PathBuf, ClewError> {
    let digest = impact_id
        .strip_prefix("thread-impact:sha256:")
        .filter(|digest| {
            digest.len() == 64
                && digest
                    .bytes()
                    .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        })
        .ok_or_else(|| invalid("thread impact id is invalid"))?;
    let directory = state.thread_root(&thread.thread_id)?.join("impacts");
    state.directory_at(&directory)?;
    Ok(directory.join(format!("{digest}.json")))
}

fn write_json_create_new<T: Serialize>(
    state: &StateAuthority,
    path: &Path,
    value: &T,
) -> Result<(), ClewError> {
    let relative = path
        .strip_prefix(state.root())
        .map_err(|_| invalid("thread impact root escapes managed state"))?;
    let parent = relative
        .parent()
        .ok_or_else(|| invalid("thread impact root has no parent"))?;
    let name = relative
        .file_name()
        .ok_or_else(|| invalid("thread impact root has no file name"))?;
    let bytes = canonical::bytes(value).map_err(internal)?;
    let directory = state.directory(parent)?;
    if directory.atomic_create(name, &bytes)? {
        return Ok(());
    }
    if directory.read_file(name, MAX_THREAD_IMPACT_ROOT_BYTES)? == bytes {
        Ok(())
    } else {
        Err(ClewError::new(
            ErrorCode::BindingChanged,
            "thread impact root was concurrently bound to different evidence",
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
        return Err(corrupt("impact source path is not repository-relative"));
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::RuntimeMode;
    use crate::session::{ModelCachePolicy, SessionAuthority, SessionLanguage};
    use crate::thread::{
        ThreadMemberBinding, ThreadStatus, TransitionTestHookPoint,
        create_with_state as create_thread_with_state, transition_with_state_for_test,
        with_transition_test_hook,
    };
    use crate::thread_callables::{
        CallableBuildInput, CallableCompilationAuthority, CallableFactSetRequest,
        CallableMemberAuthority, CallablePairBinding, CallableSelectedCompilation,
        CallableTaskBinding, GraphCoverage, KOTLIN_SEMANTIC_FACT_SCHEMA, QualifiedCallablePayload,
        RelationshipAuthority,
    };
    use crate::thread_callables_service::publish_prepared_loose_for_test;
    use std::fs;
    use std::path::PathBuf;
    use std::sync::{Arc, Barrier, mpsc};
    use std::time::Duration;

    #[derive(Clone)]
    struct StoredObject {
        reference: CasObject,
        bytes: Vec<u8>,
    }

    struct Fixture {
        state: StateAuthority,
        store: CasStore,
        thread: ThreadAuthority,
        callable_root: ThreadCallableRoot,
        fact_set: PreparedCallableFactSet,
        provider_source: StoredObject,
        provider_generation: StoredObject,
        _temporary: tempfile::TempDir,
    }

    impl Fixture {
        fn new() -> Self {
            let temporary = tempfile::tempdir().unwrap();
            let state = StateAuthority::open(temporary.path().join("v2")).unwrap();
            let store = CasStore::open(&state).unwrap();
            let provider_session = session("provider", 'a');
            let consumer_session = session("consumer", 'b');
            let thread = create_thread_with_state(
                &state,
                "thread:impact-service-fixture".into(),
                1,
                vec![
                    ThreadMemberBinding {
                        member_alias: "provider".into(),
                        service_alias: "provider-service".into(),
                        session: provider_session.clone(),
                    },
                    ThreadMemberBinding {
                        member_alias: "consumer".into(),
                        service_alias: "consumer-service".into(),
                        session: consumer_session.clone(),
                    },
                ],
            )
            .unwrap();

            let provider_source = put(
                &store,
                "codeclew-repository-source-content/1.0",
                b"fun descriptor(): String = \"provider\" // cafe\xCC\x81\n",
            );
            let consumer_source = put(
                &store,
                "codeclew-repository-source-content/1.0",
                b"fun descriptor(): String = \"consumer\" // cafe\xCC\x81\n",
            );
            let provider_snapshot = put(
                &store,
                "codeclew-repository-input-snapshot/1.0",
                b"provider-snapshot",
            );
            let consumer_snapshot = put(
                &store,
                "codeclew-repository-input-snapshot/1.0",
                b"consumer-snapshot",
            );
            let provider_generation = put(
                &store,
                "codeclew-generation-manifest/2.0",
                b"provider-generation",
            );
            let consumer_generation = put(
                &store,
                "codeclew-generation-manifest/2.0",
                b"consumer-generation",
            );

            let provider = callable_member("provider", &provider_session, &provider_snapshot);
            let consumer = callable_member("consumer", &consumer_session, &consumer_snapshot);
            let provider_compilation = compilation("provider", &provider_generation);
            let consumer_compilation = compilation("consumer", &consumer_generation);
            let provider_payload = qualified_descriptor(
                &store,
                provider.clone(),
                provider_compilation.clone(),
                "src/Provider.kt",
                0,
                &provider_source.reference,
            );
            let consumer_payload = qualified_descriptor(
                &store,
                consumer.clone(),
                consumer_compilation.clone(),
                "src/Consumer.kt",
                10,
                &consumer_source.reference,
            );
            let visited_payload_bytes = canonical::bytes(&provider_payload.payload).unwrap().len()
                + canonical::bytes(&consumer_payload.payload).unwrap().len();
            let fact_set = crate::thread_callables::build(
                CallableFactSetRequest {
                    thread_id: thread.thread_id.clone(),
                    thread_authority_digest: thread.authority_digest.clone(),
                    thread_context_id: "thread-context:impact-service-fixture".into(),
                    thread_context_authority_digest: digest("thread-context"),
                    profile_digest: digest("profile"),
                    tasks: vec![CallableTaskBinding {
                        task_id: "impact-task".into(),
                        pair_id: "provider-consumer".into(),
                        terms: vec!["publicDescriptor".into()],
                    }],
                    pairs: vec![CallablePairBinding {
                        pair_id: "provider-consumer".into(),
                        provider_member: "provider".into(),
                        consumer_member: "consumer".into(),
                        relationship_authority: RelationshipAuthority::DeclaredTopology,
                        dependency_evidence_ref: None,
                    }],
                    budgets: crate::thread_callables::CallableBudgets::frozen(),
                },
                CallableBuildInput {
                    visited_fact_count: 2,
                    visited_payload_bytes,
                    selected_compilations: vec![
                        CallableSelectedCompilation {
                            member: provider,
                            compilation: provider_compilation,
                        },
                        CallableSelectedCompilation {
                            member: consumer,
                            compilation: consumer_compilation,
                        },
                    ],
                    payloads: vec![provider_payload, consumer_payload],
                },
            )
            .unwrap();
            let callable_root =
                publish_prepared_loose_for_test(&state, &store, &thread, &fact_set).unwrap();
            Self {
                state,
                store,
                thread,
                callable_root,
                fact_set,
                provider_source,
                provider_generation,
                _temporary: temporary,
            }
        }

        fn request(&self) -> ThreadImpactServiceRequest {
            ThreadImpactServiceRequest {
                pair_id: "provider-consumer".into(),
                subject: KotlinImpactSubject::CallableFamily {
                    callable_id: "com/acme/publicDescriptor".into(),
                },
            }
        }

        fn create(&self) -> ThreadImpactRoot {
            create_with_state(
                &self.state,
                &self.thread,
                &self.callable_root.fact_set_id,
                self.request(),
            )
            .unwrap()
        }

        fn prepared_impact(&self) -> PreparedThreadImpact {
            crate::thread_impact::build_from_verified(
                &self.fact_set,
                ThreadImpactRequest {
                    fact_set_authority_digest: self
                        .callable_root
                        .authority
                        .authority_digest
                        .clone(),
                    pair_id: "provider-consumer".into(),
                    subject: self.request().subject,
                    budgets: ImpactBudgets::frozen(),
                },
            )
            .unwrap()
        }
    }

    fn session(alias: &str, seed: char) -> SessionAuthority {
        let oid = std::iter::repeat_n(seed, 40).collect::<String>();
        let mut authority = SessionAuthority {
            schema: crate::session::SESSION_SCHEMA.into(),
            authority_digest: String::new(),
            session_id: format!("session:{alias}"),
            repository_key: format!("repository:{alias}"),
            base_revision: oid.clone(),
            target_ref: "refs/heads/main".into(),
            target_oid: oid,
            runtime_key: digest("runtime"),
            runtime_mode: RuntimeMode::Development,
            language: SessionLanguage::Kotlin,
            compilations: vec![":/main".into()],
            generation_jobs: None,
            model_cache_policy: ModelCachePolicy::NonCacheable,
            model_cache_authority: None,
            created_unix_ms: 1,
        };
        let mut unsigned = authority.clone();
        unsigned.authority_digest.clear();
        authority.authority_digest = canonical::hash(&unsigned).unwrap();
        authority
    }

    fn callable_member(
        alias: &str,
        session: &SessionAuthority,
        snapshot: &StoredObject,
    ) -> CallableMemberAuthority {
        CallableMemberAuthority {
            member_alias: alias.into(),
            service_alias: format!("{alias}-service"),
            session_id: session.session_id.clone(),
            session_authority_digest: session.authority_digest.clone(),
            repository_key: session.repository_key.clone(),
            base_revision: session.base_revision.clone(),
            snapshot_ref: snapshot.reference.clone(),
        }
    }

    fn compilation(alias: &str, generation: &StoredObject) -> CallableCompilationAuthority {
        CallableCompilationAuthority {
            compilation_id: ":/main".into(),
            generation_id: digest(&format!("generation-{alias}")),
            generation_ref: generation.reference.clone(),
            semantic_authority: "K2_FIR".into(),
            extractor_id: "fir-facts-extractor/0.6".into(),
            adapter_digest: digest("adapter"),
            runtime_digest: digest("runtime"),
            descriptor_coverage: GraphCoverage::CompleteSupportedSubset,
            relation_coverage: GraphCoverage::CompleteSupportedSubset,
        }
    }

    fn qualified_descriptor(
        store: &CasStore,
        member: CallableMemberAuthority,
        compilation: CallableCompilationAuthority,
        file: &str,
        start: u64,
        source_ref: &CasObject,
    ) -> QualifiedCallablePayload {
        let payload = json!({
            "schema":"declaration-descriptor/0.1",
            "file":file,
            "start":start,
            "end":start + 8,
            "symbolIdentity":"callable:com/acme/publicDescriptor#jvm:()Ljava/lang/String;",
            "declarationKind":"FUNCTION",
            "ownerIdentity":"class:com/acme",
            "containment":["class:com/acme"],
            "visibility":"public",
            "effectiveVisibility":"public",
            "exportBoundary":"PUBLIC_API",
            "modality":"FINAL",
            "resolution":"PROVEN",
            "provider":"K2_FIR",
            "module":":app",
            "sourceSet":"main",
            "sourceProvenance":"COMPILER_UTF16_RANGE_TO_UTF8_BYTES",
            "compilerAuthority":"fir-facts-extractor/0.6",
            "typeParameters":[],
            "compilerCallableId":"com/acme/publicDescriptor",
            "isOverride":false,
            "returnType":"kotlin/String",
            "returnNullable":false,
            "parameterTypes":[],
        });
        let bytes = canonical::bytes(&payload).unwrap();
        let payload_ref = store.put(KOTLIN_SEMANTIC_FACT_SCHEMA, &bytes).unwrap();
        let fact_hash = canonical::hash_bytes(&bytes);
        QualifiedCallablePayload {
            member,
            compilation,
            fact_key: format!(
                "kotlin:descriptor:{}",
                fact_hash.strip_prefix("sha256:").unwrap()
            ),
            payload_ref,
            source_ref: Some(source_ref.clone()),
            payload,
        }
    }

    fn selected_authorities(
        fixture: &Fixture,
        alias: &str,
    ) -> (CallableMemberAuthority, CallableCompilationAuthority) {
        let member = fixture
            .fact_set
            .authority
            .members
            .iter()
            .find(|member| member.member_alias == alias)
            .unwrap();
        let compilation = member.compilations.first().unwrap();
        (
            CallableMemberAuthority {
                member_alias: member.member_alias.clone(),
                service_alias: member.service_alias.clone(),
                session_id: member.session_id.clone(),
                session_authority_digest: member.session_authority_digest.clone(),
                repository_key: member.repository_key.clone(),
                base_revision: member.base_revision.clone(),
                snapshot_ref: member.snapshot_ref.clone(),
            },
            CallableCompilationAuthority {
                compilation_id: compilation.compilation_id.clone(),
                generation_id: compilation.generation_id.clone(),
                generation_ref: compilation.generation_ref.clone(),
                semantic_authority: compilation.semantic_authority.clone(),
                extractor_id: compilation.extractor_id.clone(),
                adapter_digest: compilation.adapter_digest.clone(),
                runtime_digest: compilation.runtime_digest.clone(),
                descriptor_coverage: compilation.descriptor_coverage,
                relation_coverage: compilation.relation_coverage,
            },
        )
    }

    fn qualified_overload(
        store: &CasStore,
        member: CallableMemberAuthority,
        compilation: CallableCompilationAuthority,
        file: &str,
        start: u64,
        parameter_count: usize,
        source_ref: &CasObject,
    ) -> QualifiedCallablePayload {
        let jvm = format!(
            "({})Ljava/lang/String;",
            std::iter::repeat_n('I', parameter_count).collect::<String>()
        );
        let parameter_types =
            std::iter::repeat_n("kotlin/Int", parameter_count).collect::<Vec<_>>();
        let payload = json!({
            "schema":"declaration-descriptor/0.1",
            "file":file,
            "start":start,
            "end":start + 8,
            "symbolIdentity":format!("callable:com/acme/publicDescriptor#jvm:{jvm}"),
            "declarationKind":"FUNCTION",
            "ownerIdentity":"class:com/acme",
            "containment":["class:com/acme"],
            "visibility":"public",
            "effectiveVisibility":"public",
            "exportBoundary":"PUBLIC_API",
            "modality":"FINAL",
            "resolution":"PROVEN",
            "provider":"K2_FIR",
            "module":":app",
            "sourceSet":"main",
            "sourceProvenance":"COMPILER_UTF16_RANGE_TO_UTF8_BYTES",
            "compilerAuthority":"fir-facts-extractor/0.6",
            "typeParameters":[],
            "compilerCallableId":"com/acme/publicDescriptor",
            "isOverride":false,
            "returnType":"kotlin/String",
            "returnNullable":false,
            "parameterTypes":parameter_types,
        });
        let bytes = canonical::bytes(&payload).unwrap();
        let payload_ref = store.put(KOTLIN_SEMANTIC_FACT_SCHEMA, &bytes).unwrap();
        QualifiedCallablePayload {
            member,
            compilation,
            fact_key: format!(
                "kotlin:descriptor:{}",
                canonical::hash_bytes(&bytes)
                    .strip_prefix("sha256:")
                    .unwrap()
            ),
            payload_ref,
            source_ref: Some(source_ref.clone()),
            payload,
        }
    }

    fn large_fact_set(fixture: &Fixture, overloads_per_member: usize) -> PreparedCallableFactSet {
        let (provider, provider_compilation) = selected_authorities(fixture, "provider");
        let (consumer, consumer_compilation) = selected_authorities(fixture, "consumer");
        let source_bytes = vec![b'x'; overloads_per_member.saturating_mul(10).saturating_add(16)];
        let source = put(
            &fixture.store,
            "codeclew-repository-source-content/1.0",
            &source_bytes,
        );
        let mut payloads = Vec::with_capacity(overloads_per_member * 2);
        for (member, compilation, file) in [
            (
                provider.clone(),
                provider_compilation.clone(),
                "src/Provider.kt",
            ),
            (
                consumer.clone(),
                consumer_compilation.clone(),
                "src/Consumer.kt",
            ),
        ] {
            payloads.extend((0..overloads_per_member).map(|index| {
                qualified_overload(
                    &fixture.store,
                    member.clone(),
                    compilation.clone(),
                    file,
                    u64::try_from(index * 10).unwrap(),
                    index + 1,
                    &source.reference,
                )
            }));
        }
        let visited_payload_bytes = payloads
            .iter()
            .map(|payload| canonical::bytes(&payload.payload).unwrap().len())
            .sum();
        crate::thread_callables::build(
            CallableFactSetRequest {
                thread_id: fixture.thread.thread_id.clone(),
                thread_authority_digest: fixture.thread.authority_digest.clone(),
                thread_context_id: format!("thread-context:near-limit-{overloads_per_member}"),
                thread_context_authority_digest: digest(&format!(
                    "near-limit-context-{overloads_per_member}"
                )),
                profile_digest: digest("profile"),
                tasks: vec![CallableTaskBinding {
                    task_id: "impact-near-limit".into(),
                    pair_id: "provider-consumer".into(),
                    terms: vec!["publicDescriptor".into()],
                }],
                pairs: vec![CallablePairBinding {
                    pair_id: "provider-consumer".into(),
                    provider_member: "provider".into(),
                    consumer_member: "consumer".into(),
                    relationship_authority: RelationshipAuthority::DeclaredTopology,
                    dependency_evidence_ref: None,
                }],
                budgets: crate::thread_callables::CallableBudgets::frozen(),
            },
            CallableBuildInput {
                visited_fact_count: payloads.len(),
                visited_payload_bytes,
                selected_compilations: vec![
                    CallableSelectedCompilation {
                        member: provider,
                        compilation: provider_compilation,
                    },
                    CallableSelectedCompilation {
                        member: consumer,
                        compilation: consumer_compilation,
                    },
                ],
                payloads,
            },
        )
        .unwrap()
    }

    fn put(store: &CasStore, schema: &str, bytes: &[u8]) -> StoredObject {
        StoredObject {
            reference: store.put(schema, bytes).unwrap(),
            bytes: bytes.to_vec(),
        }
    }

    fn restore(store: &CasStore, object: &StoredObject) {
        assert_eq!(
            store
                .put(&object.reference.object_schema, &object.bytes)
                .unwrap(),
            object.reference
        );
    }

    fn digest(label: &str) -> String {
        canonical::hash(&label).unwrap()
    }

    fn loose_path(state: &StateAuthority, object: &CasObject) -> PathBuf {
        let component = object.digest.strip_prefix("sha256:").unwrap();
        state
            .objects_root()
            .join(&component[..2])
            .join(&component[2..])
    }

    fn callable_root_path(fixture: &Fixture) -> PathBuf {
        let component = fixture
            .callable_root
            .fact_set_id
            .strip_prefix("thread-callables:sha256:")
            .unwrap();
        fixture
            .state
            .thread_root(&fixture.thread.thread_id)
            .unwrap()
            .join("callable-fact-sets")
            .join(format!("{component}.json"))
    }

    #[test]
    fn concurrent_and_repeated_create_publish_one_identical_complete_root() {
        let fixture = Fixture::new();
        let barrier = Arc::new(Barrier::new(3));
        let handles = (0..2)
            .map(|_| {
                let state = fixture.state.clone();
                let thread = fixture.thread.clone();
                let fact_set_id = fixture.callable_root.fact_set_id.clone();
                let request = fixture.request();
                let barrier = Arc::clone(&barrier);
                std::thread::spawn(move || {
                    barrier.wait();
                    create_with_state(&state, &thread, &fact_set_id, request)
                })
            })
            .collect::<Vec<_>>();
        barrier.wait();
        let roots = handles
            .into_iter()
            .map(|handle| handle.join().unwrap().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(roots[0], roots[1]);
        assert_eq!(fixture.create(), roots[0]);
        let (loaded, _, prepared) = load_verified(
            &fixture.state,
            &fixture.store,
            &fixture.thread,
            &roots[0].impact_id,
        )
        .unwrap();
        assert_eq!(loaded, roots[0]);
        for reference in &prepared.authority.direct_cas_closure {
            fixture
                .store
                .read(reference, usize::try_from(reference.size).unwrap())
                .unwrap();
        }
    }

    #[test]
    fn close_wins_after_initial_open_and_resumed_create_cannot_publish() {
        let fixture = Fixture::new();
        let expected = fixture.prepared_impact();
        let expected_path = impact_root_path(
            &fixture.state,
            &fixture.thread,
            &expected.projection.impact_id,
        )
        .unwrap();
        let (reached_tx, reached_rx) = mpsc::channel();
        let (resume_tx, resume_rx) = mpsc::channel();
        let state = fixture.state.clone();
        let thread = fixture.thread.clone();
        let fact_set_id = fixture.callable_root.fact_set_id.clone();
        let request = fixture.request();
        let create_handle = std::thread::spawn(move || {
            with_create_test_hook(
                move |point| {
                    if point == CreateTestHookPoint::AfterInitialOpen {
                        reached_tx.send(()).unwrap();
                        resume_rx.recv().unwrap();
                    }
                },
                || create_with_state(&state, &thread, &fact_set_id, request),
            )
        });
        reached_rx.recv().unwrap();
        transition_with_state_for_test(&fixture.state, &fixture.thread, ThreadStatus::Closed)
            .unwrap();
        resume_tx.send(()).unwrap();
        let error = create_handle.join().unwrap().unwrap_err();
        assert_eq!(error.code, ErrorCode::PreconditionFailed);
        assert!(!fixture.state.private_file_exists(&expected_path).unwrap());
    }

    #[test]
    fn create_wins_after_admission_and_close_waits_for_a_loadable_root() {
        let fixture = Fixture::new();
        let (reached_tx, reached_rx) = mpsc::channel();
        let (resume_tx, resume_rx) = mpsc::channel();
        let state = fixture.state.clone();
        let thread = fixture.thread.clone();
        let fact_set_id = fixture.callable_root.fact_set_id.clone();
        let request = fixture.request();
        let create_handle = std::thread::spawn(move || {
            with_create_test_hook(
                move |point| {
                    if point == CreateTestHookPoint::AfterAdmission {
                        reached_tx.send(()).unwrap();
                        resume_rx.recv().unwrap();
                    }
                },
                || create_with_state(&state, &thread, &fact_set_id, request),
            )
        });
        reached_rx.recv().unwrap();

        let (close_before_tx, close_before_rx) = mpsc::channel();
        let (close_proceed_tx, close_proceed_rx) = mpsc::channel();
        let (close_acquired_tx, close_acquired_rx) = mpsc::channel();
        let (close_done_tx, close_done_rx) = mpsc::channel();
        let close_state = fixture.state.clone();
        let close_thread = fixture.thread.clone();
        let close_handle = std::thread::spawn(move || {
            let result = with_transition_test_hook(
                move |point| match point {
                    TransitionTestHookPoint::BeforeLifecycleLock => {
                        close_before_tx.send(()).unwrap();
                        close_proceed_rx.recv().unwrap();
                    }
                    TransitionTestHookPoint::AfterLifecycleLock => {
                        close_acquired_tx.send(()).unwrap();
                    }
                },
                || {
                    transition_with_state_for_test(
                        &close_state,
                        &close_thread,
                        ThreadStatus::Closed,
                    )
                },
            );
            close_done_tx.send(()).unwrap();
            result
        });
        close_before_rx.recv().unwrap();
        close_proceed_tx.send(()).unwrap();
        assert!(
            close_acquired_rx
                .recv_timeout(Duration::from_millis(100))
                .is_err()
        );

        resume_tx.send(()).unwrap();
        let root = create_handle.join().unwrap().unwrap();
        close_acquired_rx
            .recv_timeout(Duration::from_secs(5))
            .unwrap();
        close_done_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        close_handle.join().unwrap().unwrap();
        let (loaded, _, _) = load_verified(
            &fixture.state,
            &fixture.store,
            &fixture.thread,
            &root.impact_id,
        )
        .unwrap();
        assert_eq!(loaded, root);
    }

    #[test]
    fn failed_publication_never_exposes_a_partial_retained_root() {
        let fixture = Fixture::new();
        let prepared = fixture.prepared_impact();
        let root = root_from_prepared(&fixture.thread, &fixture.callable_root, &prepared).unwrap();
        let path = impact_root_path(&fixture.state, &fixture.thread, &root.impact_id).unwrap();
        fs::remove_file(loose_path(
            &fixture.state,
            &fixture.provider_generation.reference,
        ))
        .unwrap();
        assert!(
            publish_prepared(
                &fixture.state,
                &fixture.store,
                &fixture.thread,
                &prepared,
                &root,
            )
            .is_err()
        );
        assert!(!fixture.state.private_file_exists(&path).unwrap());
        restore(&fixture.store, &fixture.provider_generation);
    }

    #[test]
    fn retained_load_survives_terminal_thread_and_absent_member_state() {
        let fixture = Fixture::new();
        let root = fixture.create();
        transition_with_state_for_test(&fixture.state, &fixture.thread, ThreadStatus::Closed)
            .unwrap();
        transition_with_state_for_test(
            &fixture.state,
            &fixture.thread,
            ThreadStatus::GarbageCollected,
        )
        .unwrap();
        for member in &fixture.thread.members {
            let member_root = fixture
                .state
                .session_root(&member.session.session_id)
                .unwrap();
            fs::remove_dir_all(member_root).unwrap();
        }
        let (loaded, fact_set, impact) = load_verified(
            &fixture.state,
            &fixture.store,
            &fixture.thread,
            &root.impact_id,
        )
        .unwrap();
        assert_eq!(loaded, root);
        assert_eq!(fact_set.authority, fixture.fact_set.authority);
        assert_eq!(impact.authority, root.authority);
        for reference in &impact.authority.direct_cas_closure {
            fixture
                .store
                .read(reference, usize::try_from(reference.size).unwrap())
                .unwrap();
        }
    }

    #[test]
    fn retained_load_rejects_missing_or_tampered_parent_evidence_source_and_closure() {
        let fixture = Fixture::new();
        let root = fixture.create();
        let callable_path = callable_root_path(&fixture);
        let callable_bytes = canonical::bytes(&fixture.callable_root).unwrap();
        let impact_evidence = StoredObject {
            reference: root.authority.evidence_ref.clone(),
            bytes: fixture.prepared_impact().evidence_object.bytes,
        };
        let callable_evidence = StoredObject {
            reference: fixture.callable_root.authority.evidence_ref.clone(),
            bytes: fixture.fact_set.evidence_object.bytes.clone(),
        };

        fs::remove_file(&callable_path).unwrap();
        assert!(
            load_verified(
                &fixture.state,
                &fixture.store,
                &fixture.thread,
                &root.impact_id,
            )
            .is_err()
        );
        fixture
            .state
            .write_private_atomic(&callable_path, &callable_bytes)
            .unwrap();

        let mut substituted_parent = fixture.callable_root.clone();
        substituted_parent.thread_context_id = "thread-context:substituted".into();
        fixture
            .state
            .write_private_atomic(
                &callable_path,
                &canonical::bytes(&substituted_parent).unwrap(),
            )
            .unwrap();
        assert!(
            load_verified(
                &fixture.state,
                &fixture.store,
                &fixture.thread,
                &root.impact_id,
            )
            .is_err()
        );
        fixture
            .state
            .write_private_atomic(&callable_path, &callable_bytes)
            .unwrap();

        for missing in [
            callable_evidence.clone(),
            impact_evidence.clone(),
            fixture.provider_source.clone(),
            fixture.provider_generation.clone(),
        ] {
            fs::remove_file(loose_path(&fixture.state, &missing.reference)).unwrap();
            assert!(
                load_verified(
                    &fixture.state,
                    &fixture.store,
                    &fixture.thread,
                    &root.impact_id,
                )
                .is_err(),
                "missing {} was accepted",
                missing.reference.object_schema
            );
            restore(&fixture.store, &missing);
        }

        let evidence_path = loose_path(&fixture.state, &impact_evidence.reference);
        let mut corrupted = impact_evidence.bytes.clone();
        corrupted[0] ^= 1;
        fs::write(&evidence_path, corrupted).unwrap();
        assert!(
            load_verified(
                &fixture.state,
                &fixture.store,
                &fixture.thread,
                &root.impact_id,
            )
            .is_err()
        );
        restore(&fixture.store, &impact_evidence);
        load_verified(
            &fixture.state,
            &fixture.store,
            &fixture.thread,
            &root.impact_id,
        )
        .unwrap();
    }

    #[test]
    fn near_limit_full_service_envelope_including_outer_fields_and_lf_is_bounded() {
        let fixture = Fixture::new();
        let fact_set = large_fact_set(&fixture, 16);
        let callable_root = publish_prepared_loose_for_test(
            &fixture.state,
            &fixture.store,
            &fixture.thread,
            &fact_set,
        )
        .unwrap();
        let root = create_with_state(
            &fixture.state,
            &fixture.thread,
            &callable_root.fact_set_id,
            fixture.request(),
        )
        .unwrap();
        let value = bounded_stdout(&root).unwrap();
        assert_eq!(
            value.get("threadId").and_then(Value::as_str),
            Some(fixture.thread.thread_id.as_str())
        );
        assert_eq!(
            value.get("factSetId").and_then(Value::as_str),
            Some(callable_root.fact_set_id.as_str())
        );
        assert!(value.get("authorityDigest").is_some());
        assert!(value.get("evidenceRef").is_some());
        assert!(value.get("impact").is_some());
        let mut full_line = canonical::bytes(&value).unwrap();
        full_line.push(b'\n');
        assert!(full_line.len() <= MAX_IMPACT_STDOUT_BYTES);
        assert!(
            MAX_IMPACT_STDOUT_BYTES - full_line.len() <= 4 * 1024,
            "full managed envelope was not near its limit: {} bytes",
            full_line.len()
        );
        assert_eq!(full_line.last(), Some(&b'\n'));
        serde_json::from_slice::<Value>(&full_line[..full_line.len() - 1]).unwrap();
    }

    #[test]
    fn source_ranges_are_exact_relative_utf8_authority_and_stdout_is_one_bounded_line() {
        let fixture = Fixture::new();
        let root = fixture.create();
        let prepared = fixture.prepared_impact();
        let leases = validate_selected_sources(&fixture.store, &prepared.evidence).unwrap();
        assert!(!leases.is_empty());

        let first = prepared
            .evidence
            .selection
            .findings
            .iter()
            .find(|finding| finding.evidence.provenance.source.is_some())
            .unwrap();
        let mut unsafe_path = prepared.evidence.clone();
        unsafe_path.selection.findings[0]
            .evidence
            .provenance
            .source
            .as_mut()
            .unwrap()
            .path = "/workspace/private/Source.kt".into();
        assert!(validate_selected_sources(&fixture.store, &unsafe_path).is_err());

        let mut half_range = prepared.evidence.clone();
        half_range.selection.findings[0]
            .evidence
            .provenance
            .source
            .as_mut()
            .unwrap()
            .end = None;
        assert!(validate_selected_sources(&fixture.store, &half_range).is_err());

        let mut beyond_end = prepared.evidence.clone();
        let anchor = beyond_end.selection.findings[0]
            .evidence
            .provenance
            .source
            .as_mut()
            .unwrap();
        anchor.end = Some(anchor.content_ref.size + 1);
        assert!(validate_selected_sources(&fixture.store, &beyond_end).is_err());

        let non_utf8 = fixture
            .store
            .put("codeclew-repository-source-content/1.0", &[0xff, 0xfe])
            .unwrap();
        let mut non_utf8_source = prepared.evidence.clone();
        let anchor = non_utf8_source.selection.findings[0]
            .evidence
            .provenance
            .source
            .as_mut()
            .unwrap();
        anchor.content_ref = non_utf8;
        anchor.start = Some(0);
        anchor.end = Some(1);
        assert!(validate_selected_sources(&fixture.store, &non_utf8_source).is_err());

        let source = first.evidence.provenance.source.as_ref().unwrap();
        let source_bytes = fixture
            .store
            .read(
                &source.content_ref,
                usize::try_from(source.content_ref.size).unwrap(),
            )
            .unwrap();
        let combining = source_bytes
            .bytes()
            .windows(2)
            .position(|window| window == [0xcc, 0x81])
            .unwrap();
        let mut split_utf8 = prepared.evidence.clone();
        let anchor = split_utf8.selection.findings[0]
            .evidence
            .provenance
            .source
            .as_mut()
            .unwrap();
        anchor.start = Some(u64::try_from(combining + 1).unwrap());
        anchor.end = Some(u64::try_from(combining + 2).unwrap());
        assert!(validate_selected_sources(&fixture.store, &split_utf8).is_err());

        let value = bounded_stdout(&root).unwrap();
        let mut line = canonical::bytes(&value).unwrap();
        line.push(b'\n');
        assert!(line.len() <= MAX_IMPACT_STDOUT_BYTES);
        assert_eq!(line.last(), Some(&b'\n'));
        assert_eq!(line.iter().filter(|byte| **byte == b'\n').count(), 1);
        let rendered = String::from_utf8(line).unwrap();
        assert!(!rendered.contains("/Users/"));
        assert!(!rendered.contains("/private/"));
        assert!(!rendered.contains(fixture.state.root().to_string_lossy().as_ref()));
    }
}
