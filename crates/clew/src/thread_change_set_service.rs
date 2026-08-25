//! Managed publication and retained loading for Kotlin before/after coverage.

use crate::canonical;
use crate::cas::{CasObject, CasStore};
use crate::error::{ClewError, ErrorCode};
use crate::runtime::RuntimeAuthority;
use crate::state::StateAuthority;
use crate::thread::{ThreadAuthority, load_with_state as load_thread_with_state};
use crate::thread_callables::PreparedCasObject;
use crate::thread_callables_service;
use crate::thread_change_set::{
    ChangeSetBudgets, KOTLIN_CHANGE_COVERAGE_DOCUMENT_SCHEMA,
    KOTLIN_CHANGE_COVERAGE_EVIDENCE_SCHEMA, KotlinChangeCoverageDocument,
    KotlinChangeCoverageEvidence, MAX_CHANGE_RETAINED_CLOSURE_BYTES, MAX_CHANGE_STDOUT_BYTES,
    MemberCorrespondence, PreparedThreadChangeSet, ThreadChangeSetAuthority,
    ThreadChangeSetProjection, ThreadChangeSetRequest, ValidatorRuntimeAuthority,
    VerifiedChangeSide,
};
use crate::thread_impact_service::{self, VerifiedImpactBundle};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::path::Path;

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
    static FAIL_ROOT_PUBLICATION_AFTER_CAS: std::cell::Cell<bool> = const {
        std::cell::Cell::new(false)
    };
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

#[cfg(test)]
fn with_root_publication_failure<T>(action: impl FnOnce() -> T) -> T {
    FAIL_ROOT_PUBLICATION_AFTER_CAS.with(|flag| {
        assert!(!flag.replace(true));
    });
    let result = action();
    FAIL_ROOT_PUBLICATION_AFTER_CAS.with(|flag| flag.set(false));
    result
}

#[cfg(test)]
fn fail_root_publication_after_cas_if_requested() -> Result<(), ClewError> {
    if FAIL_ROOT_PUBLICATION_AFTER_CAS.with(std::cell::Cell::get) {
        Err(internal(
            "injected failure after coverage CAS batch and before root publication",
        ))
    } else {
        Ok(())
    }
}

pub const THREAD_CHANGE_SET_ROOT_SCHEMA: &str = "codeclew-thread-change-coverage-root/1.0";
pub const THREAD_CHANGE_SET_RESULT_SCHEMA: &str = "codeclew-thread-change-coverage-result/1.0";
const MAX_THREAD_CHANGE_SET_ROOT_BYTES: usize = 65 * 1024 * 1024;

#[derive(Debug, Clone)]
pub struct ThreadChangeSetServiceRequest {
    pub before_impact_id: String,
    pub after_impact_id: String,
    pub member_correspondence: Vec<MemberCorrespondence>,
    pub coverage_document: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ThreadChangeSetRoot {
    pub schema: String,
    pub change_set_id: String,
    pub before_thread_id: String,
    pub before_thread_authority_digest: String,
    pub after_thread_id: String,
    pub after_thread_authority_digest: String,
    pub authority: ThreadChangeSetAuthority,
    pub projection: ThreadChangeSetProjection,
}

pub fn create(
    before: &ThreadAuthority,
    after: &ThreadAuthority,
    request: ThreadChangeSetServiceRequest,
) -> Result<ThreadChangeSetRoot, ClewError> {
    let state = StateAuthority::process_default()?;
    create_with_state(&state, before, after, request)
}

pub(crate) fn create_with_state(
    state: &StateAuthority,
    before: &ThreadAuthority,
    after: &ThreadAuthority,
    request: ThreadChangeSetServiceRequest,
) -> Result<ThreadChangeSetRoot, ClewError> {
    let runtime = required_runtime()?;
    let validator = validator_runtime(&runtime);
    create_with_state_and_validator(state, before, after, request, validator, || {
        revalidate_runtime(&runtime)
    })
}

fn create_with_state_and_validator(
    state: &StateAuthority,
    before: &ThreadAuthority,
    after: &ThreadAuthority,
    request: ThreadChangeSetServiceRequest,
    validator_runtime: ValidatorRuntimeAuthority,
    revalidate_validator: impl FnOnce() -> Result<(), ClewError>,
) -> Result<ThreadChangeSetRoot, ClewError> {
    before.verify()?;
    after.verify()?;
    after.require_open_with_state(state)?;
    #[cfg(test)]
    invoke_create_test_hook(CreateTestHookPoint::AfterInitialOpen);
    let store = CasStore::open(state)?;
    let before_bundle = thread_impact_service::load_verified_bundle(
        state,
        &store,
        before,
        &request.before_impact_id,
    )?;
    let after_bundle = thread_impact_service::load_verified_bundle(
        state,
        &store,
        after,
        &request.after_impact_id,
    )?;
    let prepared = crate::thread_change_set::build_from_verified(
        verified_side(before, &before_bundle),
        verified_side(after, &after_bundle),
        ThreadChangeSetRequest {
            member_correspondence: request.member_correspondence,
            coverage_document: request.coverage_document,
            validator_runtime,
            budgets: ChangeSetBudgets::frozen(),
        },
    )?;
    crate::thread_change_set::verify_prepared_from_verified(
        verified_side(before, &before_bundle),
        verified_side(after, &after_bundle),
        &prepared,
    )?;
    let root = root_from_prepared(before, after, &prepared)?;
    bounded_stdout(&root)?;
    verify_closure_objects(&store, &prepared, false)?;

    // Comparison and all resource preflights stay outside the lifecycle lock.
    // Publication alone is linearized against close on the after thread.
    let _admission = after.admit_with_state(state)?;
    #[cfg(test)]
    invoke_create_test_hook(CreateTestHookPoint::AfterAdmission);
    revalidate_validator()?;
    crate::thread::revalidate_authority_record(state, before)?;
    crate::thread::revalidate_authority_record(state, after)?;
    thread_callables_service::revalidate_root_record(state, before, &before_bundle.callable_root)?;
    thread_callables_service::revalidate_root_record(state, after, &after_bundle.callable_root)?;
    thread_impact_service::revalidate_root_record(state, before, &before_bundle.impact_root)?;
    thread_impact_service::revalidate_root_record(state, after, &after_bundle.impact_root)?;
    publish_prepared(state, &store, after, &prepared, &root)?;
    Ok(root)
}

pub fn load(
    after: &ThreadAuthority,
    change_set_id: &str,
) -> Result<ThreadChangeSetRoot, ClewError> {
    let state = StateAuthority::process_default()?;
    let store = CasStore::open(&state)?;
    load_verified(&state, &store, after, change_set_id).map(|bundle| bundle.root)
}

struct VerifiedChangeSetBundle {
    root: ThreadChangeSetRoot,
    _before_thread: ThreadAuthority,
    _before_impact: VerifiedImpactBundle,
    _after_impact: VerifiedImpactBundle,
    _prepared: PreparedThreadChangeSet,
}

fn load_verified(
    state: &StateAuthority,
    store: &CasStore,
    after: &ThreadAuthority,
    change_set_id: &str,
) -> Result<VerifiedChangeSetBundle, ClewError> {
    after.verify()?;
    let path = change_set_root_path(state, after, change_set_id)?;
    let bytes = state
        .read_private_file(&path, MAX_THREAD_CHANGE_SET_ROOT_BYTES)
        .map_err(|_| invalid("thread coverage root is missing or exceeds 65 MiB"))?;
    let root: ThreadChangeSetRoot =
        serde_json::from_slice(&bytes).map_err(|_| corrupt("thread coverage root is invalid"))?;
    if canonical::bytes(&root).map_err(internal)? != bytes
        || root.change_set_id != change_set_id
        || root.after_thread_id != after.thread_id
        || root.after_thread_authority_digest != after.authority_digest
    {
        return Err(corrupt(
            "thread coverage retained root authority is invalid",
        ));
    }
    validate_root(&root)?;
    let (before, _) = load_thread_with_state(state, &root.before_thread_id)?;
    if before.authority_digest != root.before_thread_authority_digest {
        return Err(corrupt("thread coverage before-thread authority changed"));
    }
    let before_bundle = thread_impact_service::load_verified_bundle(
        state,
        store,
        &before,
        &root.authority.before.impact_id,
    )?;
    let after_bundle = thread_impact_service::load_verified_bundle(
        state,
        store,
        after,
        &root.authority.after.impact_id,
    )?;
    let coverage_document_object = read_prepared_object(
        store,
        &root.authority.coverage_document_ref,
        KOTLIN_CHANGE_COVERAGE_DOCUMENT_SCHEMA,
    )?;
    let coverage_document: KotlinChangeCoverageDocument =
        serde_json::from_slice(&coverage_document_object.bytes)
            .map_err(|_| corrupt("thread coverage document is invalid"))?;
    if canonical::bytes(&coverage_document).map_err(internal)? != coverage_document_object.bytes {
        return Err(corrupt("thread coverage document is not canonical"));
    }
    let evidence_object = read_prepared_object(
        store,
        &root.authority.evidence_ref,
        KOTLIN_CHANGE_COVERAGE_EVIDENCE_SCHEMA,
    )?;
    let evidence: KotlinChangeCoverageEvidence = serde_json::from_slice(&evidence_object.bytes)
        .map_err(|_| corrupt("thread coverage evidence is invalid"))?;
    if canonical::bytes(&evidence).map_err(internal)? != evidence_object.bytes {
        return Err(corrupt("thread coverage evidence is not canonical"));
    }
    let prepared = PreparedThreadChangeSet {
        authority: root.authority.clone(),
        evidence,
        coverage_document,
        coverage_document_object,
        evidence_object,
        authority_bytes: canonical::bytes(&root.authority).map_err(internal)?,
        projection: root.projection.clone(),
        projection_bytes: canonical::bytes(&root.projection).map_err(internal)?,
    };
    crate::thread_change_set::verify_prepared_from_verified(
        verified_side(&before, &before_bundle),
        verified_side(after, &after_bundle),
        &prepared,
    )?;
    verify_closure_objects(store, &prepared, true)?;
    Ok(VerifiedChangeSetBundle {
        root,
        _before_thread: before,
        _before_impact: before_bundle,
        _after_impact: after_bundle,
        _prepared: prepared,
    })
}

pub fn bounded_stdout(root: &ThreadChangeSetRoot) -> Result<Value, ClewError> {
    validate_root(root)?;
    let value = json!({
        "schema":THREAD_CHANGE_SET_RESULT_SCHEMA,
        "changeSetId":root.change_set_id,
        "authorityDigest":root.authority.authority_digest,
        "beforeThreadId":root.before_thread_id,
        "beforeThreadAuthorityDigest":root.before_thread_authority_digest,
        "beforeImpactId":root.authority.before.impact_id,
        "afterThreadId":root.after_thread_id,
        "afterThreadAuthorityDigest":root.after_thread_authority_digest,
        "afterImpactId":root.authority.after.impact_id,
        "coverage":root.projection,
    });
    let bytes = canonical::bytes(&value).map_err(internal)?;
    enforce_serialized_bound(
        bytes.len().saturating_add(1),
        MAX_CHANGE_STDOUT_BYTES,
        "thread coverage stdout exceeds 64 KiB",
    )?;
    Ok(value)
}

fn verified_side<'a>(
    thread: &'a ThreadAuthority,
    bundle: &'a VerifiedImpactBundle,
) -> VerifiedChangeSide<'a> {
    VerifiedChangeSide {
        thread,
        fact_set: &bundle.fact_set,
        impact: &bundle.impact,
    }
}

fn required_runtime() -> Result<RuntimeAuthority, ClewError> {
    RuntimeAuthority::from_environment()?.ok_or_else(|| {
        ClewError::new(
            ErrorCode::PreconditionFailed,
            "thread coverage requires the managed ./clew runtime capsule",
        )
    })
}

fn validator_runtime(runtime: &RuntimeAuthority) -> ValidatorRuntimeAuthority {
    ValidatorRuntimeAuthority {
        runtime_key: runtime.runtime_key.clone(),
        runtime_mode: runtime.mode,
        manifest_digest: runtime.manifest_digest.clone(),
    }
}

fn revalidate_runtime(expected: &RuntimeAuthority) -> Result<(), ClewError> {
    let actual = required_runtime()?;
    if actual.runtime_key != expected.runtime_key
        || actual.mode != expected.mode
        || actual.manifest_digest != expected.manifest_digest
    {
        return Err(ClewError::new(
            ErrorCode::BindingChanged,
            "validator runtime changed before thread coverage publication",
        ));
    }
    Ok(())
}

fn root_from_prepared(
    before: &ThreadAuthority,
    after: &ThreadAuthority,
    prepared: &PreparedThreadChangeSet,
) -> Result<ThreadChangeSetRoot, ClewError> {
    let root = ThreadChangeSetRoot {
        schema: THREAD_CHANGE_SET_ROOT_SCHEMA.into(),
        change_set_id: prepared.projection.change_set_id.clone(),
        before_thread_id: before.thread_id.clone(),
        before_thread_authority_digest: before.authority_digest.clone(),
        after_thread_id: after.thread_id.clone(),
        after_thread_authority_digest: after.authority_digest.clone(),
        authority: prepared.authority.clone(),
        projection: prepared.projection.clone(),
    };
    validate_root(&root)?;
    Ok(root)
}

fn validate_root(root: &ThreadChangeSetRoot) -> Result<(), ClewError> {
    enforce_serialized_bound(
        canonical::bytes(root).map_err(internal)?.len(),
        MAX_THREAD_CHANGE_SET_ROOT_BYTES,
        "thread coverage retained root exceeds 65 MiB",
    )?;
    if root.schema != THREAD_CHANGE_SET_ROOT_SCHEMA
        || root.change_set_id != format!("thread-coverage:{}", root.authority.authority_digest)
        || root.before_thread_id != root.authority.before.thread_id
        || root.before_thread_authority_digest != root.authority.before.thread_authority_digest
        || root.after_thread_id != root.authority.after.thread_id
        || root.after_thread_authority_digest != root.authority.after.thread_authority_digest
        || root.projection != crate::thread_change_set::project(&root.authority)
        || root.projection.change_set_id != root.change_set_id
    {
        return Err(corrupt(
            "thread coverage retained root authority is invalid",
        ));
    }
    Ok(())
}

fn enforce_serialized_bound(
    actual: usize,
    maximum: usize,
    message: &'static str,
) -> Result<(), ClewError> {
    if actual > maximum {
        Err(budget(message))
    } else {
        Ok(())
    }
}

fn publish_prepared(
    state: &StateAuthority,
    store: &CasStore,
    after: &ThreadAuthority,
    prepared: &PreparedThreadChangeSet,
    root: &ThreadChangeSetRoot,
) -> Result<(), ClewError> {
    let path = change_set_root_path(state, after, &root.change_set_id)?;
    let root_bytes = canonical::bytes(root).map_err(internal)?;
    if state.private_file_exists(&path)? {
        let existing = state.read_private_file(&path, MAX_THREAD_CHANGE_SET_ROOT_BYTES)?;
        if existing != root_bytes {
            return Err(ClewError::new(
                ErrorCode::BindingChanged,
                "thread coverage identifier is already bound to different evidence",
            ));
        }
        verify_closure_objects(store, prepared, true)?;
        return Ok(());
    }
    let published = store.put_batch(vec![
        (
            KOTLIN_CHANGE_COVERAGE_DOCUMENT_SCHEMA.into(),
            prepared.coverage_document_object.bytes.clone(),
        ),
        (
            KOTLIN_CHANGE_COVERAGE_EVIDENCE_SCHEMA.into(),
            prepared.evidence_object.bytes.clone(),
        ),
    ])?;
    if published
        != vec![
            prepared.coverage_document_object.reference.clone(),
            prepared.evidence_object.reference.clone(),
        ]
    {
        return Err(corrupt(
            "CAS batch publication returned another thread coverage identity",
        ));
    }
    verify_closure_objects(store, prepared, true)?;
    #[cfg(test)]
    fail_root_publication_after_cas_if_requested()?;
    write_json_create_new(state, &path, root)
}

fn verify_closure_objects(
    store: &CasStore,
    prepared: &PreparedThreadChangeSet,
    derived_published: bool,
) -> Result<(), ClewError> {
    let mut total = 0usize;
    for reference in &prepared.authority.direct_cas_closure {
        let size = usize::try_from(reference.size)
            .map_err(|_| budget("thread coverage retained object exceeds host size"))?;
        total = total
            .checked_add(size)
            .ok_or_else(|| budget("thread coverage retained byte count overflowed"))?;
        let derived = reference == &prepared.coverage_document_object.reference
            || reference == &prepared.evidence_object.reference;
        if !derived || derived_published {
            store.read(reference, size)?;
        }
    }
    if total != prepared.authority.retained_cas_bytes
        || total > MAX_CHANGE_RETAINED_CLOSURE_BYTES
        || prepared.authority.new_derived_cas_object_count != 2
        || !prepared
            .authority
            .direct_cas_closure
            .contains(&prepared.coverage_document_object.reference)
        || !prepared
            .authority
            .direct_cas_closure
            .contains(&prepared.evidence_object.reference)
    {
        return Err(corrupt(
            "thread coverage retained CAS closure is inconsistent",
        ));
    }
    Ok(())
}

fn read_prepared_object(
    store: &CasStore,
    reference: &CasObject,
    expected_schema: &str,
) -> Result<PreparedCasObject, ClewError> {
    if reference.object_schema != expected_schema {
        return Err(corrupt("thread coverage CAS object has another schema"));
    }
    let size = usize::try_from(reference.size)
        .map_err(|_| budget("thread coverage CAS object exceeds host size"))?;
    if size > MAX_CHANGE_RETAINED_CLOSURE_BYTES {
        return Err(budget("thread coverage CAS object exceeds 64 MiB"));
    }
    let lease = store.read(reference, size)?;
    Ok(PreparedCasObject {
        reference: reference.clone(),
        bytes: lease.bytes().to_vec(),
    })
}

fn change_set_root_path(
    state: &StateAuthority,
    after: &ThreadAuthority,
    change_set_id: &str,
) -> Result<std::path::PathBuf, ClewError> {
    let digest = change_set_id
        .strip_prefix("thread-coverage:sha256:")
        .filter(|digest| {
            digest.len() == 64
                && digest
                    .bytes()
                    .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        })
        .ok_or_else(|| invalid("thread coverage id is invalid"))?;
    let directory = state.thread_root(&after.thread_id)?.join("change-sets");
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
        .map_err(|_| invalid("thread coverage root escapes managed state"))?;
    let parent = relative
        .parent()
        .ok_or_else(|| invalid("thread coverage root has no parent"))?;
    let name = relative
        .file_name()
        .ok_or_else(|| invalid("thread coverage root has no file name"))?;
    let bytes = canonical::bytes(value).map_err(internal)?;
    let directory = state.directory(parent)?;
    if directory.atomic_create(name, &bytes)? {
        return Ok(());
    }
    if directory.read_file(name, MAX_THREAD_CHANGE_SET_ROOT_BYTES)? == bytes {
        Ok(())
    } else {
        Err(ClewError::new(
            ErrorCode::BindingChanged,
            "thread coverage root was concurrently bound to different evidence",
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
    use crate::thread_change_set::{
        ChangeSetStatus, CoverageDocumentEntry, CoverageHandling, CoverageHandlingKind,
        KotlinChangeCoverageDocument,
    };
    use crate::thread_impact::KotlinImpactSubject;
    use crate::thread_impact_service::{ThreadImpactRoot, ThreadImpactServiceRequest};
    use std::fs;
    use std::sync::{Arc, Barrier, mpsc};
    use std::time::Duration;

    struct SideFixture {
        thread: ThreadAuthority,
        impact_root: ThreadImpactRoot,
    }

    struct Fixture {
        state: StateAuthority,
        store: CasStore,
        before: SideFixture,
        after: SideFixture,
        validator: ValidatorRuntimeAuthority,
        _temporary: tempfile::TempDir,
    }

    impl Fixture {
        fn new() -> Self {
            let temporary = tempfile::tempdir().unwrap();
            let state = StateAuthority::open(temporary.path().join("v2")).unwrap();
            let store = CasStore::open(&state).unwrap();
            let before = side(
                &state,
                &store,
                "before",
                "provider-old",
                "consumer-old",
                "kotlin/String",
                "()Ljava/lang/String;",
                'a',
            );
            let after = side(
                &state,
                &store,
                "after",
                "provider-new",
                "consumer-new",
                "kotlin/Int",
                "()I",
                'c',
            );
            Self {
                state,
                store,
                before,
                after,
                validator: ValidatorRuntimeAuthority {
                    runtime_key: digest("runtime"),
                    runtime_mode: RuntimeMode::Development,
                    manifest_digest: digest("manifest"),
                },
                _temporary: temporary,
            }
        }

        fn correspondence(&self) -> Vec<MemberCorrespondence> {
            vec![
                MemberCorrespondence {
                    before_member_alias: "provider-old".into(),
                    after_member_alias: "provider-new".into(),
                },
                MemberCorrespondence {
                    before_member_alias: "consumer-old".into(),
                    after_member_alias: "consumer-new".into(),
                },
            ]
        }

        fn request(&self, coverage_document: Vec<u8>) -> ThreadChangeSetServiceRequest {
            ThreadChangeSetServiceRequest {
                before_impact_id: self.before.impact_root.impact_id.clone(),
                after_impact_id: self.after.impact_root.impact_id.clone(),
                member_correspondence: self.correspondence(),
                coverage_document,
            }
        }

        fn create(&self, coverage_document: Vec<u8>) -> ThreadChangeSetRoot {
            create_with_state_and_validator(
                &self.state,
                &self.before.thread,
                &self.after.thread,
                self.request(coverage_document),
                self.validator.clone(),
                || Ok(()),
            )
            .unwrap()
        }

        fn prepared(&self, coverage_document: Vec<u8>) -> PreparedThreadChangeSet {
            let before_bundle = thread_impact_service::load_verified_bundle(
                &self.state,
                &self.store,
                &self.before.thread,
                &self.before.impact_root.impact_id,
            )
            .unwrap();
            let after_bundle = thread_impact_service::load_verified_bundle(
                &self.state,
                &self.store,
                &self.after.thread,
                &self.after.impact_root.impact_id,
            )
            .unwrap();
            crate::thread_change_set::build_from_verified(
                verified_side(&self.before.thread, &before_bundle),
                verified_side(&self.after.thread, &after_bundle),
                ThreadChangeSetRequest {
                    member_correspondence: self.correspondence(),
                    coverage_document,
                    validator_runtime: self.validator.clone(),
                    budgets: ChangeSetBudgets::frozen(),
                },
            )
            .unwrap()
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn side(
        state: &StateAuthority,
        store: &CasStore,
        phase: &str,
        provider_alias: &str,
        consumer_alias: &str,
        provider_return_type: &str,
        provider_jvm_descriptor: &str,
        seed: char,
    ) -> SideFixture {
        let provider_session = session(phase, "provider", provider_alias, seed);
        let consumer_session = session(
            phase,
            "consumer",
            consumer_alias,
            char::from_u32(seed as u32 + 1).unwrap(),
        );
        let unrelated_alias = format!("unrelated-{phase}");
        let unrelated_session = session(
            phase,
            "unrelated",
            &unrelated_alias,
            char::from_u32(seed as u32 + 2).unwrap(),
        );
        let thread = create_thread_with_state(
            state,
            format!("thread:coverage-{phase}"),
            1,
            vec![
                ThreadMemberBinding {
                    member_alias: provider_alias.into(),
                    service_alias: "provider-service".into(),
                    session: provider_session.clone(),
                },
                ThreadMemberBinding {
                    member_alias: consumer_alias.into(),
                    service_alias: "consumer-service".into(),
                    session: consumer_session.clone(),
                },
                ThreadMemberBinding {
                    member_alias: unrelated_alias,
                    service_alias: "unrelated-service".into(),
                    session: unrelated_session,
                },
            ],
        )
        .unwrap();
        let provider_source = put(
            store,
            "codeclew-repository-source-content/1.0",
            format!("fun publicDescriptor(): Any = \"{phase}\"\n").as_bytes(),
        );
        let consumer_source = put(
            store,
            "codeclew-repository-source-content/1.0",
            b"fun publicDescriptor(): String = \"consumer\"\n",
        );
        let provider_snapshot = put(
            store,
            "codeclew-repository-input-snapshot/1.0",
            format!("provider-{phase}-snapshot").as_bytes(),
        );
        let consumer_snapshot = put(
            store,
            "codeclew-repository-input-snapshot/1.0",
            format!("consumer-{phase}-snapshot").as_bytes(),
        );
        let provider_generation = put(
            store,
            "codeclew-generation-manifest/2.0",
            format!("provider-{phase}-generation").as_bytes(),
        );
        let consumer_generation = put(
            store,
            "codeclew-generation-manifest/2.0",
            format!("consumer-{phase}-generation").as_bytes(),
        );
        let provider_member = callable_member(
            provider_alias,
            "provider-service",
            &provider_session,
            &provider_snapshot,
        );
        let consumer_member = callable_member(
            consumer_alias,
            "consumer-service",
            &consumer_session,
            &consumer_snapshot,
        );
        let provider_compilation = compilation(phase, "provider", &provider_generation);
        let consumer_compilation = compilation(phase, "consumer", &consumer_generation);
        let provider_payload = qualified_descriptor(
            store,
            provider_member.clone(),
            provider_compilation.clone(),
            "src/Provider.kt",
            &provider_source,
            provider_return_type,
            provider_jvm_descriptor,
        );
        let consumer_payload = qualified_descriptor(
            store,
            consumer_member.clone(),
            consumer_compilation.clone(),
            "src/Consumer.kt",
            &consumer_source,
            "kotlin/String",
            "()Ljava/lang/String;",
        );
        let visited_payload_bytes = canonical::bytes(&provider_payload.payload).unwrap().len()
            + canonical::bytes(&consumer_payload.payload).unwrap().len();
        let fact_set = crate::thread_callables::build(
            CallableFactSetRequest {
                thread_id: thread.thread_id.clone(),
                thread_authority_digest: thread.authority_digest.clone(),
                thread_context_id: format!("thread-context:coverage-{phase}"),
                thread_context_authority_digest: digest(&format!("context-{phase}")),
                profile_digest: digest("profile"),
                tasks: vec![CallableTaskBinding {
                    task_id: "coverage-task".into(),
                    pair_id: "provider-consumer".into(),
                    terms: vec!["publicDescriptor".into()],
                }],
                pairs: vec![CallablePairBinding {
                    pair_id: "provider-consumer".into(),
                    provider_member: provider_alias.into(),
                    consumer_member: consumer_alias.into(),
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
                        member: provider_member,
                        compilation: provider_compilation,
                    },
                    CallableSelectedCompilation {
                        member: consumer_member,
                        compilation: consumer_compilation,
                    },
                ],
                payloads: vec![provider_payload, consumer_payload],
            },
        )
        .unwrap();
        let callable_root =
            publish_prepared_loose_for_test(state, store, &thread, &fact_set).unwrap();
        let impact_root = thread_impact_service::create_with_state(
            state,
            &thread,
            &callable_root.fact_set_id,
            ThreadImpactServiceRequest {
                pair_id: "provider-consumer".into(),
                subject: KotlinImpactSubject::CallableFamily {
                    callable_id: "com/acme/publicDescriptor".into(),
                },
            },
        )
        .unwrap();
        SideFixture {
            thread,
            impact_root,
        }
    }

    fn session(phase: &str, repository: &str, alias: &str, seed: char) -> SessionAuthority {
        let oid = std::iter::repeat_n(seed, 40).collect::<String>();
        let mut authority = SessionAuthority {
            schema: crate::session::SESSION_SCHEMA.into(),
            authority_digest: String::new(),
            session_id: format!("session:{phase}-{alias}"),
            repository_key: format!("repository:{repository}"),
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
        service_alias: &str,
        session: &SessionAuthority,
        snapshot_ref: &CasObject,
    ) -> CallableMemberAuthority {
        CallableMemberAuthority {
            member_alias: alias.into(),
            service_alias: service_alias.into(),
            session_id: session.session_id.clone(),
            session_authority_digest: session.authority_digest.clone(),
            repository_key: session.repository_key.clone(),
            base_revision: session.base_revision.clone(),
            snapshot_ref: snapshot_ref.clone(),
        }
    }

    fn compilation(
        phase: &str,
        role: &str,
        generation_ref: &CasObject,
    ) -> CallableCompilationAuthority {
        CallableCompilationAuthority {
            compilation_id: ":/main".into(),
            generation_id: digest(&format!("generation-{phase}-{role}")),
            generation_ref: generation_ref.clone(),
            semantic_authority: "K2_FIR".into(),
            extractor_id: "fir-facts-extractor/0.6".into(),
            adapter_digest: digest("adapter"),
            runtime_digest: digest("runtime"),
            descriptor_coverage: GraphCoverage::CompleteSupportedSubset,
            relation_coverage: GraphCoverage::CompleteSupportedSubset,
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn qualified_descriptor(
        store: &CasStore,
        member: CallableMemberAuthority,
        compilation: CallableCompilationAuthority,
        file: &str,
        source_ref: &CasObject,
        return_type: &str,
        jvm_descriptor: &str,
    ) -> QualifiedCallablePayload {
        let payload = json!({
            "schema":"declaration-descriptor/0.1",
            "file":file,
            "start":0,
            "end":8,
            "symbolIdentity":format!("callable:com/acme/publicDescriptor#jvm:{jvm_descriptor}"),
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
            "returnType":return_type,
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

    fn put(store: &CasStore, schema: &str, bytes: &[u8]) -> CasObject {
        store.put(schema, bytes).unwrap()
    }

    fn digest(label: &str) -> String {
        canonical::hash_bytes(label.as_bytes())
    }

    fn empty_coverage() -> Vec<u8> {
        canonical::bytes(&KotlinChangeCoverageDocument {
            schema: KOTLIN_CHANGE_COVERAGE_DOCUMENT_SCHEMA.into(),
            entries: Vec::new(),
        })
        .unwrap()
    }

    fn complete_coverage(root: &ThreadChangeSetRoot) -> Vec<u8> {
        canonical::bytes(&KotlinChangeCoverageDocument {
            schema: KOTLIN_CHANGE_COVERAGE_DOCUMENT_SCHEMA.into(),
            entries: root
                .projection
                .missing_targets
                .iter()
                .enumerate()
                .map(|(index, target)| CoverageDocumentEntry {
                    target_id: target.target_id.clone(),
                    required_categories: target.required_categories.clone(),
                    handling: CoverageHandling {
                        kind: CoverageHandlingKind::ExternalWork,
                        id: format!("verify-{index}"),
                    },
                })
                .collect(),
        })
        .unwrap()
    }

    #[test]
    fn managed_incomplete_complete_repeat_and_terminal_retention_are_exact() {
        let fixture = Fixture::new();
        let incomplete = fixture.create(empty_coverage());
        assert_eq!(incomplete.authority.status, ChangeSetStatus::Incomplete);
        assert!(incomplete.authority.missing_target_count > 0);
        assert_eq!(fixture.create(empty_coverage()), incomplete);
        let output = bounded_stdout(&incomplete).unwrap();
        assert!(canonical::bytes(&output).unwrap().len() < MAX_CHANGE_STDOUT_BYTES);

        let complete = fixture.create(complete_coverage(&incomplete));
        assert_eq!(
            complete.authority.status,
            ChangeSetStatus::ValidatedConditional
        );
        assert_eq!(complete.authority.missing_target_count, 0);
        assert_eq!(
            complete.authority.comparison_digest,
            incomplete.authority.comparison_digest
        );
        assert_ne!(complete.change_set_id, incomplete.change_set_id);
        assert_eq!(fixture.create(complete_coverage(&incomplete)), complete);
        assert_eq!(complete.authority.new_derived_cas_object_count, 2);

        transition_with_state_for_test(
            &fixture.state,
            &fixture.before.thread,
            ThreadStatus::Closed,
        )
        .unwrap();
        transition_with_state_for_test(
            &fixture.state,
            &fixture.before.thread,
            ThreadStatus::GarbageCollected,
        )
        .unwrap();
        transition_with_state_for_test(&fixture.state, &fixture.after.thread, ThreadStatus::Closed)
            .unwrap();
        transition_with_state_for_test(
            &fixture.state,
            &fixture.after.thread,
            ThreadStatus::GarbageCollected,
        )
        .unwrap();
        let loaded = load_verified(
            &fixture.state,
            &fixture.store,
            &fixture.after.thread,
            &complete.change_set_id,
        )
        .unwrap();
        assert_eq!(loaded.root, complete);
        for reference in &complete.authority.direct_cas_closure {
            fixture
                .store
                .read(reference, usize::try_from(reference.size).unwrap())
                .unwrap();
        }
    }

    #[test]
    fn stdout_and_retained_root_bounds_are_exact() {
        for maximum in [MAX_CHANGE_STDOUT_BYTES, MAX_THREAD_CHANGE_SET_ROOT_BYTES] {
            assert!(enforce_serialized_bound(maximum, maximum, "bounded").is_ok());
            assert_eq!(
                enforce_serialized_bound(maximum + 1, maximum, "bounded")
                    .unwrap_err()
                    .code,
                ErrorCode::SliceBudgetExceeded
            );
        }
    }

    #[test]
    fn comparison_identity_survives_validator_capsule_rebuild() {
        let fixture = Fixture::new();
        let first = fixture.create(empty_coverage());
        let rebuilt_validator = ValidatorRuntimeAuthority {
            runtime_key: digest("rebuilt-runtime"),
            runtime_mode: RuntimeMode::Development,
            manifest_digest: digest("rebuilt-manifest"),
        };
        let second = create_with_state_and_validator(
            &fixture.state,
            &fixture.before.thread,
            &fixture.after.thread,
            fixture.request(empty_coverage()),
            rebuilt_validator,
            || Ok(()),
        )
        .unwrap();
        assert_eq!(
            first.authority.comparison_digest,
            second.authority.comparison_digest
        );
        assert_eq!(
            first.projection.missing_targets,
            second.projection.missing_targets
        );
        assert_ne!(
            first.authority.authority_digest,
            second.authority.authority_digest
        );
        assert_ne!(first.change_set_id, second.change_set_id);
    }

    #[test]
    fn correspondence_is_exactly_the_selected_pair_even_in_larger_threads() {
        let fixture = Fixture::new();
        let accepted = fixture.create(empty_coverage());
        assert_eq!(accepted.authority.member_target_count, 2);
        assert_eq!(fixture.before.thread.members.len(), 3);
        assert_eq!(fixture.after.thread.members.len(), 3);

        let directory = fixture
            .state
            .thread_root(&fixture.after.thread.thread_id)
            .unwrap()
            .join("change-sets");
        let roots_before = fs::read_dir(&directory).unwrap().count();
        let mut request = fixture.request(empty_coverage());
        request.member_correspondence.push(MemberCorrespondence {
            before_member_alias: "unrelated-before".into(),
            after_member_alias: "unrelated-after".into(),
        });
        let error = create_with_state_and_validator(
            &fixture.state,
            &fixture.before.thread,
            &fixture.after.thread,
            request,
            fixture.validator.clone(),
            || Ok(()),
        )
        .unwrap_err();
        assert_eq!(error.code, ErrorCode::SliceBudgetExceeded);
        assert_eq!(fs::read_dir(directory).unwrap().count(), roots_before);
    }

    #[test]
    fn coverage_entry_budget_failure_publishes_neither_document_nor_root() {
        let fixture = Fixture::new();
        let document = KotlinChangeCoverageDocument {
            schema: KOTLIN_CHANGE_COVERAGE_DOCUMENT_SCHEMA.into(),
            entries: (0..=crate::thread_change_set::MAX_CHANGE_COVERAGE_ENTRIES)
                .map(|index| CoverageDocumentEntry {
                    target_id: digest(&format!("overflow-target-{index}")),
                    required_categories: vec![
                        crate::thread_change_set::VerificationCategory::StructuralComparison,
                    ],
                    handling: CoverageHandling {
                        kind: CoverageHandlingKind::Action,
                        id: format!("a{index}"),
                    },
                })
                .collect(),
        };
        let bytes = canonical::bytes(&document).unwrap();
        assert!(bytes.len() < crate::thread_change_set::MAX_CHANGE_COVERAGE_DOCUMENT_BYTES);
        let document_ref =
            CasObject::for_bytes(KOTLIN_CHANGE_COVERAGE_DOCUMENT_SCHEMA, &bytes).unwrap();
        assert!(fixture.store.read(&document_ref, bytes.len()).is_err());
        let error = create_with_state_and_validator(
            &fixture.state,
            &fixture.before.thread,
            &fixture.after.thread,
            fixture.request(bytes.clone()),
            fixture.validator.clone(),
            || Ok(()),
        )
        .unwrap_err();
        assert_eq!(error.code, ErrorCode::SliceBudgetExceeded);
        assert!(fixture.store.read(&document_ref, bytes.len()).is_err());
        let directory = fixture
            .state
            .thread_root(&fixture.after.thread.thread_id)
            .unwrap()
            .join("change-sets");
        assert!(!directory.exists() || fs::read_dir(directory).unwrap().next().is_none());
    }

    #[test]
    fn prepared_parent_result_and_cas_substitution_fail_closed() {
        let fixture = Fixture::new();
        let prepared = fixture.prepared(empty_coverage());
        let before_bundle = thread_impact_service::load_verified_bundle(
            &fixture.state,
            &fixture.store,
            &fixture.before.thread,
            &fixture.before.impact_root.impact_id,
        )
        .unwrap();
        let after_bundle = thread_impact_service::load_verified_bundle(
            &fixture.state,
            &fixture.store,
            &fixture.after.thread,
            &fixture.after.impact_root.impact_id,
        )
        .unwrap();
        let before = verified_side(&fixture.before.thread, &before_bundle);
        let after = verified_side(&fixture.after.thread, &after_bundle);

        assert!(
            crate::thread_change_set::verify_prepared_from_verified(after, after, &prepared)
                .is_err()
        );
        let mut result_substitution = prepared.clone();
        result_substitution.projection.status = ChangeSetStatus::ValidatedConditional;
        assert!(
            crate::thread_change_set::verify_prepared_from_verified(
                before,
                after,
                &result_substitution
            )
            .is_err()
        );
        let mut cas_substitution = prepared;
        cas_substitution.evidence_object.bytes.push(b' ');
        assert!(
            crate::thread_change_set::verify_prepared_from_verified(
                before,
                after,
                &cas_substitution
            )
            .is_err()
        );
    }

    #[test]
    fn injected_failure_after_cas_batch_leaves_no_retained_root() {
        let fixture = Fixture::new();
        let prepared = fixture.prepared(empty_coverage());
        for object in [
            &prepared.coverage_document_object.reference,
            &prepared.evidence_object.reference,
        ] {
            assert!(
                fixture
                    .store
                    .read(object, usize::try_from(object.size).unwrap())
                    .is_err()
            );
        }
        let root =
            root_from_prepared(&fixture.before.thread, &fixture.after.thread, &prepared).unwrap();
        let root_path =
            change_set_root_path(&fixture.state, &fixture.after.thread, &root.change_set_id)
                .unwrap();
        let error = with_root_publication_failure(|| {
            create_with_state_and_validator(
                &fixture.state,
                &fixture.before.thread,
                &fixture.after.thread,
                fixture.request(empty_coverage()),
                fixture.validator.clone(),
                || Ok(()),
            )
        })
        .unwrap_err();
        assert_eq!(error.code, ErrorCode::Internal);
        assert!(!root_path.exists());
        for object in [
            &prepared.coverage_document_object.reference,
            &prepared.evidence_object.reference,
        ] {
            fixture
                .store
                .read(object, usize::try_from(object.size).unwrap())
                .unwrap();
        }
    }

    #[test]
    fn before_may_be_terminal_before_validation_but_after_must_be_open() {
        let fixture = Fixture::new();
        transition_with_state_for_test(
            &fixture.state,
            &fixture.before.thread,
            ThreadStatus::Closed,
        )
        .unwrap();
        transition_with_state_for_test(
            &fixture.state,
            &fixture.before.thread,
            ThreadStatus::GarbageCollected,
        )
        .unwrap();
        assert_eq!(
            fixture.create(empty_coverage()).authority.status,
            ChangeSetStatus::Incomplete
        );
    }

    #[test]
    fn concurrent_repeat_publishes_one_identical_complete_root() {
        let fixture = Fixture::new();
        let barrier = Arc::new(Barrier::new(3));
        let handles = (0..2)
            .map(|_| {
                let state = fixture.state.clone();
                let before = fixture.before.thread.clone();
                let after = fixture.after.thread.clone();
                let request = fixture.request(empty_coverage());
                let validator = fixture.validator.clone();
                let barrier = Arc::clone(&barrier);
                std::thread::spawn(move || {
                    barrier.wait();
                    create_with_state_and_validator(
                        &state,
                        &before,
                        &after,
                        request,
                        validator,
                        || Ok(()),
                    )
                })
            })
            .collect::<Vec<_>>();
        barrier.wait();
        let roots = handles
            .into_iter()
            .map(|handle| handle.join().unwrap().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(roots[0], roots[1]);
    }

    #[test]
    fn close_wins_before_admission_and_no_root_is_published() {
        let fixture = Fixture::new();
        let (reached_tx, reached_rx) = mpsc::channel();
        let (resume_tx, resume_rx) = mpsc::channel();
        let state = fixture.state.clone();
        let before = fixture.before.thread.clone();
        let after = fixture.after.thread.clone();
        let request = fixture.request(empty_coverage());
        let validator = fixture.validator.clone();
        let create_handle = std::thread::spawn(move || {
            with_create_test_hook(
                move |point| {
                    if point == CreateTestHookPoint::AfterInitialOpen {
                        reached_tx.send(()).unwrap();
                        resume_rx.recv().unwrap();
                    }
                },
                || {
                    create_with_state_and_validator(
                        &state,
                        &before,
                        &after,
                        request,
                        validator,
                        || Ok(()),
                    )
                },
            )
        });
        reached_rx.recv().unwrap();
        transition_with_state_for_test(&fixture.state, &fixture.after.thread, ThreadStatus::Closed)
            .unwrap();
        resume_tx.send(()).unwrap();
        assert_eq!(
            create_handle.join().unwrap().unwrap_err().code,
            ErrorCode::PreconditionFailed
        );
        let directory = fixture
            .state
            .thread_root(&fixture.after.thread.thread_id)
            .unwrap()
            .join("change-sets");
        assert!(!directory.exists() || fs::read_dir(directory).unwrap().next().is_none());
    }

    #[test]
    fn validation_wins_after_admission_and_close_waits_for_loadable_root() {
        let fixture = Fixture::new();
        let (reached_tx, reached_rx) = mpsc::channel();
        let (resume_tx, resume_rx) = mpsc::channel();
        let state = fixture.state.clone();
        let before = fixture.before.thread.clone();
        let after = fixture.after.thread.clone();
        let request = fixture.request(empty_coverage());
        let validator = fixture.validator.clone();
        let create_handle = std::thread::spawn(move || {
            with_create_test_hook(
                move |point| {
                    if point == CreateTestHookPoint::AfterAdmission {
                        reached_tx.send(()).unwrap();
                        resume_rx.recv().unwrap();
                    }
                },
                || {
                    create_with_state_and_validator(
                        &state,
                        &before,
                        &after,
                        request,
                        validator,
                        || Ok(()),
                    )
                },
            )
        });
        reached_rx.recv().unwrap();
        let (close_before_tx, close_before_rx) = mpsc::channel();
        let (close_proceed_tx, close_proceed_rx) = mpsc::channel();
        let (close_acquired_tx, close_acquired_rx) = mpsc::channel();
        let close_state = fixture.state.clone();
        let close_thread = fixture.after.thread.clone();
        let close_handle = std::thread::spawn(move || {
            with_transition_test_hook(
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
            )
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
        close_handle.join().unwrap().unwrap();
        assert_eq!(
            load_verified(
                &fixture.state,
                &fixture.store,
                &fixture.after.thread,
                &root.change_set_id,
            )
            .unwrap()
            .root,
            root
        );
    }

    #[test]
    fn malformed_unknown_and_shell_shaped_coverage_fail_without_new_root() {
        let fixture = Fixture::new();
        let incomplete = fixture.create(empty_coverage());
        let directory = fixture
            .state
            .thread_root(&fixture.after.thread.thread_id)
            .unwrap()
            .join("change-sets");
        let count = fs::read_dir(&directory).unwrap().count();
        let target = &incomplete.projection.missing_targets[0];
        for bytes in [
            br#"{"schema":"codeclew-kotlin-change-coverage-document/1.0","entries":[],"command":"rm"}"#.to_vec(),
            canonical::bytes(&KotlinChangeCoverageDocument {
                schema: KOTLIN_CHANGE_COVERAGE_DOCUMENT_SCHEMA.into(),
                entries: vec![CoverageDocumentEntry {
                    target_id: target.target_id.clone(),
                    required_categories: target.required_categories.clone(),
                    handling: CoverageHandling {
                        kind: CoverageHandlingKind::Action,
                        id: "run;command".into(),
                    },
                }],
            })
            .unwrap(),
            canonical::bytes(&KotlinChangeCoverageDocument {
                schema: KOTLIN_CHANGE_COVERAGE_DOCUMENT_SCHEMA.into(),
                entries: vec![CoverageDocumentEntry {
                    target_id:
                        "sha256:ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"
                            .into(),
                    required_categories: target.required_categories.clone(),
                    handling: CoverageHandling {
                        kind: CoverageHandlingKind::ExternalWork,
                        id: "unknown-target".into(),
                    },
                }],
            })
            .unwrap(),
        ] {
            assert!(
                create_with_state_and_validator(
                    &fixture.state,
                    &fixture.before.thread,
                    &fixture.after.thread,
                    fixture.request(bytes),
                    fixture.validator.clone(),
                    || Ok(()),
                )
                .is_err()
            );
        }
        assert_eq!(fs::read_dir(directory).unwrap().count(), count);
    }
}
