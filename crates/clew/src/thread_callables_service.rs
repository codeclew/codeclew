use crate::adapter_v2::FactRecord;
use crate::canonical;
use crate::cas::{CAS_OBJECT_SCHEMA, CasObject, CasStore};
use crate::error::{ClewError, ErrorCode};
use crate::generation_service::{
    AnalysisExecutionAuthority, ReadyGeneration, ReadyGenerationSet, load_session_generation,
    load_snapshot,
};
use crate::generation_v2::{GENERATION_SCHEMA, GenerationManifest};
use crate::kotlin_adapter_v2::{KOTLIN_FACTS_CAPABILITY, kotlin_adapter_digest};
use crate::repository_snapshot::{RepositoryInputSnapshot, WorktreeKind};
use crate::runtime::RuntimeAuthority;
use crate::semantic_validation::{KotlinSemanticPayloadKind, validate_kotlin_semantic_payload};
use crate::session::{ContextObject, SessionAdmission, SessionAuthority, SessionLanguage};
use crate::state::StateAuthority;
use crate::thread::{ThreadAuthority, ThreadMemberBinding};
use crate::thread_callables::{
    CALLABLE_FACT_SET_SCHEMA, CallableBudgets, CallableBuildInput, CallableCompilationAuthority,
    CallableFactSetAuthority, CallableFactSetEvidence, CallableFactSetProjection,
    CallableFactSetRequest, CallableMemberAuthority, CallablePairBinding,
    CallableQueryIndexManifest, CallableSelectedCompilation, CallableTaskBinding, GraphCoverage,
    KOTLIN_SEMANTIC_FACT_SCHEMA, MAX_CALLABLE_EVIDENCE_OBJECT_BYTES, MAX_CALLABLE_SHARD_BYTES,
    MAX_INPUT_PAYLOAD_BYTES, MAX_SELECTED_SOURCE_BYTES, PreparedCallableFactSet, PreparedCasObject,
    QualifiedCallablePayload, RelationshipAuthority, validate_direct_cas_closure_size,
};
use crate::thread_context::{MAX_THREAD_STDOUT_BYTES, ThreadContextObject};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path};

pub const THREAD_CALLABLE_ROOT_SCHEMA: &str = "codeclew-thread-callable-root/1.0";
pub const THREAD_CALLABLE_RESULT_SCHEMA: &str = "codeclew-thread-callables-result/1.0";
const MAX_THREAD_CALLABLE_ROOT_BYTES: usize = 65 * 1024 * 1024;
const EXTRACTOR_AUTHORITY: &str = "fir-facts-extractor/0.6";
const SEMANTIC_AUTHORITY: &str = "K2_FIR";

#[derive(Debug, Clone)]
pub struct ThreadCallablesRequest {
    pub task_id: String,
    pub pair_id: String,
    pub provider_member: String,
    pub consumer_member: String,
    pub terms: Vec<String>,
}

/// Keep one public product entry while selecting the strongest profile the two
/// bound language units can honestly support.
pub fn create_bounded(
    thread: &ThreadAuthority,
    context_id: &str,
    request: ThreadCallablesRequest,
) -> Result<Value, ClewError> {
    let selected = selected_members(thread, &request)?;
    if selected
        .iter()
        .all(|member| member.session.language == SessionLanguage::Kotlin)
    {
        let root = create(thread, context_id, request)?;
        return bounded_stdout(&root);
    }
    if selected.len() == 2
        && selected
            .iter()
            .any(|member| member.session.language == SessionLanguage::Kotlin)
        && selected
            .iter()
            .any(|member| member.session.language == SessionLanguage::Java)
    {
        return crate::jvm_navigation_service::create(thread, context_id, request);
    }
    Err(ClewError::new(
        ErrorCode::UnsupportedLanguage,
        "thread callables supports Kotlin pairs or one Kotlin/Java pair",
    ))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ThreadCallableRoot {
    pub schema: String,
    pub fact_set_id: String,
    pub thread_id: String,
    pub thread_authority_digest: String,
    pub thread_context_id: String,
    pub thread_context_authority_digest: String,
    pub authority: CallableFactSetAuthority,
    pub projection: CallableFactSetProjection,
}

#[derive(Debug, Clone)]
struct CapturedMember {
    binding: ThreadMemberBinding,
    session: SessionAuthority,
    context: ContextObject,
    ready: ReadyGenerationSet,
}

#[derive(Debug)]
struct PendingPayload {
    fact: FactRecord,
    source_ref: Option<CasObject>,
    payload: Value,
}

pub fn create(
    thread: &ThreadAuthority,
    context_id: &str,
    request: ThreadCallablesRequest,
) -> Result<ThreadCallableRoot, ClewError> {
    thread.verify()?;
    let request = validate_request(thread, request)?;
    let context = ThreadContextObject::load(thread, context_id)?;
    if context.authority.thread_id != thread.thread_id
        || context.authority.thread_authority_digest != thread.authority_digest
    {
        return Err(invalid(
            "thread context belongs to another thread authority",
        ));
    }

    let state = StateAuthority::process_default()?;
    thread.require_open_with_state(&state)?;
    let store = CasStore::open(&state)?;
    let runtime = RuntimeAuthority::from_environment()?.ok_or_else(|| {
        ClewError::new(
            ErrorCode::PreconditionFailed,
            "thread callables requires the managed ./clew runtime capsule",
        )
    })?;
    let selected = selected_members(thread, &request)?;
    let mut captured = Vec::with_capacity(selected.len());
    let mut payloads = Vec::new();
    let mut selected_compilations = Vec::new();
    let mut source_verifier = SourceVerifier::default();
    let mut visited_fact_count = 0usize;
    let mut visited_payload_bytes = 0usize;
    for binding in selected {
        let member_context = context
            .authority
            .members
            .iter()
            .find(|candidate| candidate.member_alias == binding.member_alias)
            .ok_or_else(|| invalid("thread context omits a selected member"))?;
        let (session, _) = SessionAuthority::load(&binding.session.session_id)?;
        if canonical::bytes(&session).map_err(internal)?
            != canonical::bytes(&binding.session).map_err(internal)?
            || session.language != SessionLanguage::Kotlin
            || member_context.session_authority_digest != session.authority_digest
            || member_context.language != session.language.uri()
        {
            return Err(ClewError::new(
                ErrorCode::PreconditionFailed,
                "thread callable member is stale, substituted, or not Kotlin",
            ));
        }
        session.require_open()?;
        if session.runtime_key != runtime.runtime_key {
            return Err(ClewError::new(
                ErrorCode::ProjectModelChanged,
                "thread callable member runtime differs from the active capsule",
            ));
        }
        let ready = load_session_generation(&session)?;
        if selected_compilations
            .len()
            .checked_add(ready.compilations.len())
            .is_none_or(|count| count > CallableBudgets::frozen().max_compilations)
        {
            return Err(ClewError::new(
                ErrorCode::SliceBudgetExceeded,
                "thread callable selected compilations exceed 64",
            ));
        }
        let member_single_context = session.load_context(&member_context.context_id)?;
        validate_member_context(member_context, &session, &member_single_context, &ready)?;
        let snapshot = load_snapshot(&store, &ready)?;
        let source_map = EffectiveSourceMap::new(&snapshot)?;
        let member_authority = CallableMemberAuthority {
            member_alias: binding.member_alias.clone(),
            service_alias: binding.service_alias.clone(),
            session_id: session.session_id.clone(),
            session_authority_digest: session.authority_digest.clone(),
            repository_key: session.repository_key.clone(),
            base_revision: session.base_revision.clone(),
            snapshot_ref: ready.repository_snapshot.clone(),
        };
        for ready_compilation in &ready.compilations {
            let (compilation, facts, fact_count, payload_bytes) = collect_compilation_payloads(
                &store,
                &runtime,
                &source_map,
                &mut source_verifier,
                ready_compilation,
            )?;
            visited_fact_count = checked_add_budget(
                visited_fact_count,
                fact_count,
                CallableBudgets::frozen().max_input_facts_visited,
                "thread callable input fact count exceeds 131072",
            )?;
            visited_payload_bytes = checked_add_budget(
                visited_payload_bytes,
                payload_bytes,
                CallableBudgets::frozen().max_input_payload_bytes,
                "thread callable input payload bytes exceed 32 MiB",
            )?;
            selected_compilations.push(CallableSelectedCompilation {
                member: member_authority.clone(),
                compilation: compilation.clone(),
            });
            payloads.extend(facts.into_iter().map(|pending| QualifiedCallablePayload {
                member: member_authority.clone(),
                compilation: compilation.clone(),
                fact_key: pending.fact.fact_key,
                payload_ref: pending.fact.payload,
                source_ref: pending.source_ref,
                payload: pending.payload,
            }));
        }
        captured.push(CapturedMember {
            binding: binding.clone(),
            session,
            context: member_single_context,
            ready,
        });
    }
    if payloads.is_empty() {
        return Err(ClewError::new(
            ErrorCode::IncompleteSemanticAnalysis,
            "selected members have no qualified K2 descriptor evidence",
        ));
    }

    let prepared = crate::thread_callables::build(
        CallableFactSetRequest {
            thread_id: thread.thread_id.clone(),
            thread_authority_digest: thread.authority_digest.clone(),
            thread_context_id: context.context_id.clone(),
            thread_context_authority_digest: context.authority.authority_digest.clone(),
            profile_digest: profile_digest()?,
            tasks: vec![CallableTaskBinding {
                task_id: request.task_id,
                pair_id: request.pair_id.clone(),
                terms: request.terms,
            }],
            pairs: vec![CallablePairBinding {
                pair_id: request.pair_id,
                provider_member: request.provider_member,
                consumer_member: request.consumer_member,
                relationship_authority: RelationshipAuthority::DeclaredTopology,
                dependency_evidence_ref: None,
            }],
            budgets: CallableBudgets::frozen(),
        },
        CallableBuildInput {
            visited_fact_count,
            visited_payload_bytes,
            selected_compilations,
            payloads,
        },
    )?;
    crate::thread_callables::verify_prepared(&prepared)?;
    let root = root_from_prepared(thread, &context, &prepared)?;
    bounded_stdout(&root)?;

    // Member admissions exclude close/abort while the captured generation
    // bindings are re-read. The thread admission is acquired last and remains
    // held through CAS and retained-root publication.
    let _member_admissions = captured
        .iter()
        .map(|member| member.session.open_admission())
        .collect::<Result<Vec<SessionAdmission>, ClewError>>()?;
    revalidate_captured(&state, &context, &captured)?;
    let _thread_admission = thread.admit_with_state(&state)?;
    revalidate_persisted_thread(&state, thread)?;
    let reloaded_context = ThreadContextObject::load(thread, &context.context_id)?;
    if canonical::bytes(&reloaded_context).map_err(internal)?
        != canonical::bytes(&context).map_err(internal)?
    {
        return Err(ClewError::new(
            ErrorCode::BindingChanged,
            "thread context authority changed before callable publication",
        ));
    }
    publish_prepared(&state, &store, thread, &prepared, &root)?;
    Ok(root)
}

pub fn bounded_stdout(root: &ThreadCallableRoot) -> Result<Value, ClewError> {
    validate_root(root)?;
    let value = json!({
        "schema":THREAD_CALLABLE_RESULT_SCHEMA,
        "threadId":root.thread_id,
        "threadAuthorityDigest":root.thread_authority_digest,
        "contextId":root.thread_context_id,
        "contextAuthorityDigest":root.thread_context_authority_digest,
        "factSetId":root.fact_set_id,
        "authorityDigest":root.authority.authority_digest,
        "evidenceRef":root.authority.evidence_ref,
        "queryIndexRef":root.authority.query_index_ref,
        "callables":root.projection,
    });
    if canonical::bytes(&value)
        .map_err(internal)?
        .len()
        .saturating_add(1)
        > MAX_THREAD_STDOUT_BYTES
    {
        return Err(ClewError::new(
            ErrorCode::SliceBudgetExceeded,
            "thread callable stdout exceeds 64 KiB",
        ));
    }
    Ok(value)
}

pub fn load(thread: &ThreadAuthority, fact_set_id: &str) -> Result<ThreadCallableRoot, ClewError> {
    let state = StateAuthority::process_default()?;
    let store = CasStore::open(&state)?;
    load_verified(&state, &store, thread, fact_set_id).map(|(root, _prepared)| root)
}

pub(crate) fn load_verified(
    state: &StateAuthority,
    store: &CasStore,
    thread: &ThreadAuthority,
    fact_set_id: &str,
) -> Result<(ThreadCallableRoot, PreparedCallableFactSet), ClewError> {
    thread.verify()?;
    let root_path = callable_root_path(state, thread, fact_set_id)?;
    let bytes = state
        .read_private_file(&root_path, MAX_THREAD_CALLABLE_ROOT_BYTES)
        .map_err(|_| invalid("thread callable root is missing or exceeds 65 MiB"))?;
    let root: ThreadCallableRoot =
        serde_json::from_slice(&bytes).map_err(|_| corrupt("thread callable root is invalid"))?;
    if canonical::bytes(&root).map_err(internal)? != bytes
        || root.fact_set_id != fact_set_id
        || root.thread_id != thread.thread_id
        || root.thread_authority_digest != thread.authority_digest
    {
        return Err(corrupt("thread callable root authority is invalid"));
    }
    validate_root(&root)?;
    validate_retained_closure_size(&root.authority)?;
    let prepared = load_prepared_from_root(store, &root)?;
    verify_retained_closure(store, &root.authority)?;
    Ok((root, prepared))
}

pub(crate) fn revalidate_root_record(
    state: &StateAuthority,
    thread: &ThreadAuthority,
    expected: &ThreadCallableRoot,
) -> Result<(), ClewError> {
    let root_path = callable_root_path(state, thread, &expected.fact_set_id)?;
    let bytes = state
        .read_private_file(&root_path, MAX_THREAD_CALLABLE_ROOT_BYTES)
        .map_err(|_| corrupt("thread callable root disappeared before derived publication"))?;
    if bytes != canonical::bytes(expected).map_err(internal)? {
        return Err(ClewError::new(
            ErrorCode::BindingChanged,
            "thread callable root changed before derived publication",
        ));
    }
    Ok(())
}

fn validate_request(
    thread: &ThreadAuthority,
    mut request: ThreadCallablesRequest,
) -> Result<ThreadCallablesRequest, ClewError> {
    for (value, label) in [
        (&request.task_id, "task id"),
        (&request.pair_id, "pair id"),
        (&request.provider_member, "provider member"),
        (&request.consumer_member, "consumer member"),
    ] {
        validate_identifier(value, label)?;
    }
    if request.provider_member == request.consumer_member {
        return Err(invalid("thread callable pair cannot bind one member twice"));
    }
    if request.terms.is_empty() || request.terms.len() > 256 {
        return Err(invalid("thread callable request requires 1-256 terms"));
    }
    for term in &request.terms {
        if term.trim().is_empty()
            || term.len() > 4096
            || term.chars().any(char::is_control)
            || !crate::text_authority::is_nfc(term)
        {
            return Err(invalid("thread callable term is not bounded NFC text"));
        }
    }
    request.terms.sort();
    request.terms.dedup();
    let aliases = thread
        .members
        .iter()
        .map(|member| member.member_alias.as_str())
        .collect::<BTreeSet<_>>();
    if !aliases.contains(request.provider_member.as_str())
        || !aliases.contains(request.consumer_member.as_str())
    {
        return Err(invalid("thread callable pair names an unknown member"));
    }
    Ok(request)
}

fn selected_members<'a>(
    thread: &'a ThreadAuthority,
    request: &ThreadCallablesRequest,
) -> Result<Vec<&'a ThreadMemberBinding>, ClewError> {
    let selected = thread
        .members
        .iter()
        .filter(|member| {
            member.member_alias == request.provider_member
                || member.member_alias == request.consumer_member
        })
        .collect::<Vec<_>>();
    if selected.len() != 2 {
        return Err(invalid("thread callable pair binding is incomplete"));
    }
    Ok(selected)
}

fn validate_member_context(
    binding: &crate::thread_context::ThreadMemberContextBinding,
    session: &SessionAuthority,
    context: &ContextObject,
    ready: &ReadyGenerationSet,
) -> Result<(), ClewError> {
    if context.session_id != session.session_id
        || context.session_authority_digest != session.authority_digest
        || canonical::hash(context).map_err(internal)? != binding.context_digest
        || context.evidence_digest != binding.evidence_digest
        || context.evidence_ref != binding.evidence_ref
        || ready.runtime_key != session.runtime_key
        || ready.base_revision != session.base_revision
        || ready.repository_snapshot
            != context_cas(
                &context.evidence,
                "/context/snapshot/repositorySnapshot",
                crate::repository_snapshot::SNAPSHOT_SCHEMA,
            )?
    {
        return Err(ClewError::new(
            ErrorCode::BindingChanged,
            "member context and ready generation authorities differ",
        ));
    }
    let rows = context
        .evidence
        .pointer("/context/snapshot/compilations")
        .and_then(Value::as_array)
        .ok_or_else(|| corrupt("member context has no compilation authority"))?;
    if rows.len() != ready.compilations.len() {
        return Err(ClewError::new(
            ErrorCode::BindingChanged,
            "member context compilation authority is incomplete",
        ));
    }
    for (row, compilation) in rows.iter().zip(&ready.compilations) {
        if row.get("compilation").and_then(Value::as_str) != Some(compilation.compilation.as_str())
            || row.get("compilerVersion").and_then(Value::as_str)
                != Some(compilation.compiler_version.as_str())
            || context_cas_value(row.get("generation"), GENERATION_SCHEMA)?
                != compilation.generation
            || context_cas_value(row.get("queryIndex"), crate::query_v2::QUERY_INDEX_SCHEMA)?
                != compilation.query_index
        {
            return Err(ClewError::new(
                ErrorCode::BindingChanged,
                "member context generation reference changed",
            ));
        }
    }
    Ok(())
}

fn collect_compilation_payloads(
    store: &CasStore,
    runtime: &RuntimeAuthority,
    sources: &EffectiveSourceMap,
    source_verifier: &mut SourceVerifier,
    ready: &ReadyGeneration,
) -> Result<
    (
        CallableCompilationAuthority,
        Vec<PendingPayload>,
        usize,
        usize,
    ),
    ClewError,
> {
    if ready.incremental.analysis_execution_authority != AnalysisExecutionAuthority::CompilerWorker
    {
        return Err(ClewError::new(
            ErrorCode::IncompleteSemanticAnalysis,
            "Kotlin callable profile rejects syntax-only generation authority",
        ));
    }
    let generation: GenerationManifest = read_canonical_cas(store, &ready.generation)?;
    if generation.derived_input_manifest != ready.derived_input_manifest {
        return Err(corrupt(
            "generation is bound to another derived input manifest",
        ));
    }
    generation.verify(store)?;
    if generation.attempts.len() != 1
        || generation.attempts[0].capability.as_str() != KOTLIN_FACTS_CAPABILITY
    {
        return Err(ClewError::new(
            ErrorCode::IncompleteSemanticAnalysis,
            "generation lacks one sealed Kotlin semantic-facts attempt",
        ));
    }
    let adapter_digest = adapter_digest(runtime, &ready.compiler_version)?;
    let mut descriptor_coverage = GraphCoverage::CompleteSupportedSubset;
    let mut relation_coverage = GraphCoverage::CompleteSupportedSubset;
    let mut selected = Vec::new();
    let mut payload_bytes = 0usize;
    generation.visit_facts(store, |fact| {
        payload_bytes = checked_add_budget(
            payload_bytes,
            usize::try_from(fact.payload.size)
                .map_err(|_| budget("semantic payload exceeds host size"))?,
            MAX_INPUT_PAYLOAD_BYTES,
            "thread callable input payload bytes exceed 32 MiB",
        )?;
        if fact.domain_uri.as_str() != KOTLIN_FACTS_CAPABILITY {
            return Err(corrupt(
                "Kotlin generation contains another capability domain",
            ));
        }
        let Some(category) = kotlin_fact_category(&fact.fact_key)? else {
            return Ok(());
        };
        if fact.payload.object_schema != KOTLIN_SEMANTIC_FACT_SCHEMA {
            return Err(corrupt("Kotlin semantic fact payload schema changed"));
        }
        let payload: Value = read_canonical_cas(store, &fact.payload)?;
        let kind = validate_kotlin_semantic_payload(&payload)?;
        if !category_matches_kind(category, kind) {
            return Err(corrupt("Kotlin fact category and payload schema differ"));
        }
        if payload
            .get("code")
            .and_then(Value::as_str)
            .is_some_and(|code| code.contains("SYNTAX_ONLY") || code.contains("SOURCE_SYNTAX"))
        {
            return Err(ClewError::new(
                ErrorCode::IncompleteSemanticAnalysis,
                "Kotlin callable profile rejects syntax-only boundary evidence",
            ));
        }
        let source_ref = source_verifier.bind(store, sources, &payload, kind)?;
        match kind {
            KotlinSemanticPayloadKind::DeclarationDescriptor => {
                if payload.get("attributeCoverage").is_some() {
                    descriptor_coverage = GraphCoverage::Partial;
                }
            }
            KotlinSemanticPayloadKind::DeclarationRelation => {
                if payload.get("attributeCoverage").is_some() {
                    relation_coverage = GraphCoverage::Partial;
                }
            }
            KotlinSemanticPayloadKind::DeclarationDescriptorBoundary => {
                descriptor_coverage = GraphCoverage::Partial;
            }
            KotlinSemanticPayloadKind::DeclarationRelationBoundary => {
                relation_coverage = GraphCoverage::Partial;
            }
        }
        selected.push(PendingPayload {
            fact: fact.clone(),
            source_ref,
            payload,
        });
        Ok(())
    })?;
    let authority = CallableCompilationAuthority {
        compilation_id: ready.compilation.clone(),
        generation_id: generation.generation_id,
        generation_ref: ready.generation.clone(),
        semantic_authority: SEMANTIC_AUTHORITY.into(),
        extractor_id: EXTRACTOR_AUTHORITY.into(),
        adapter_digest,
        runtime_digest: ready.runtime_key.clone(),
        descriptor_coverage,
        relation_coverage,
    };
    let fact_count = usize::try_from(generation.fact_count)
        .map_err(|_| budget("generation fact count exceeds host size"))?;
    Ok((authority, selected, fact_count, payload_bytes))
}

#[derive(Debug, Default)]
struct SourceVerifier {
    cached: BTreeMap<(String, String), (CasObject, Vec<u8>)>,
    cached_bytes: usize,
}

impl SourceVerifier {
    fn bind(
        &mut self,
        store: &CasStore,
        sources: &EffectiveSourceMap,
        payload: &Value,
        kind: KotlinSemanticPayloadKind,
    ) -> Result<Option<CasObject>, ClewError> {
        let Some(path) = payload.get("file").and_then(Value::as_str) else {
            if matches!(
                kind,
                KotlinSemanticPayloadKind::DeclarationDescriptor
                    | KotlinSemanticPayloadKind::DeclarationRelation
            ) {
                return Err(corrupt("proven Kotlin semantic fact has no source file"));
            }
            return Ok(None);
        };
        validate_relative_source(path)?;
        let source = sources
            .get(path)
            .ok_or_else(|| corrupt("semantic fact source is absent from the sealed snapshot"))?
            .clone();
        let key = (source.object_schema.clone(), source.digest.clone());
        if let Some((existing, _)) = self.cached.get(&key)
            && existing != &source
        {
            return Err(corrupt(
                "source CAS identity repeats with conflicting metadata",
            ));
        }
        if !self.cached.contains_key(&key) {
            let size =
                usize::try_from(source.size).map_err(|_| budget("source exceeds host size"))?;
            self.cached_bytes = checked_add_budget(
                self.cached_bytes,
                size,
                MAX_SELECTED_SOURCE_BYTES,
                "selected Kotlin source authority exceeds 64 MiB",
            )?;
            let lease = store.read(&source, size)?;
            std::str::from_utf8(lease.bytes())
                .map_err(|_| corrupt("Kotlin source blob is not UTF-8"))?;
            self.cached
                .insert(key.clone(), (source.clone(), lease.bytes().to_vec()));
        }
        let bytes = &self.cached.get(&key).expect("source was inserted above").1;
        let text = std::str::from_utf8(bytes)
            .expect("cached Kotlin source was validated as UTF-8 before insertion");
        if let (Some(start), Some(end)) = (
            payload.get("start").and_then(Value::as_u64),
            payload.get("end").and_then(Value::as_u64),
        ) {
            let start =
                usize::try_from(start).map_err(|_| corrupt("source start exceeds host size"))?;
            let end = usize::try_from(end).map_err(|_| corrupt("source end exceeds host size"))?;
            if start > end
                || end > text.len()
                || !text.is_char_boundary(start)
                || !text.is_char_boundary(end)
            {
                return Err(corrupt(
                    "semantic fact range is outside its exact source CAS object",
                ));
            }
        } else if payload.get("start").is_some() || payload.get("end").is_some() {
            return Err(corrupt("semantic fact has only one source range endpoint"));
        }
        Ok(Some(source))
    }
}

#[derive(Debug)]
struct EffectiveSourceMap {
    index: BTreeMap<String, CasObject>,
    worktree: BTreeMap<String, (WorktreeKind, Option<CasObject>)>,
}

impl EffectiveSourceMap {
    fn new(snapshot: &RepositoryInputSnapshot) -> Result<Self, ClewError> {
        snapshot.verify()?;
        let mut index = BTreeMap::new();
        for entry in &snapshot.index {
            if entry.stage == 0
                && index
                    .insert(entry.path.clone(), entry.content.clone())
                    .is_some()
            {
                return Err(corrupt("snapshot repeats a stage-zero source path"));
            }
        }
        let mut worktree = BTreeMap::new();
        for entry in &snapshot.worktree {
            if worktree
                .insert(entry.path.clone(), (entry.kind, entry.content.clone()))
                .is_some()
            {
                return Err(corrupt("snapshot repeats a worktree source path"));
            }
        }
        Ok(Self { index, worktree })
    }

    fn get(&self, path: &str) -> Option<&CasObject> {
        match self.worktree.get(path) {
            Some((WorktreeKind::Regular, Some(content))) => Some(content),
            Some((WorktreeKind::Regular, None))
            | Some((WorktreeKind::Missing, _))
            | Some((WorktreeKind::Symlink, _)) => None,
            None => self.index.get(path),
        }
    }
}

fn publish_prepared(
    state: &StateAuthority,
    store: &CasStore,
    thread: &ThreadAuthority,
    prepared: &PreparedCallableFactSet,
    root: &ThreadCallableRoot,
) -> Result<(), ClewError> {
    validate_retained_closure_size(&root.authority)?;
    let root_path = callable_root_path(state, thread, &root.fact_set_id)?;
    if state.private_file_exists(&root_path)? {
        let existing = state.read_private_file(&root_path, MAX_THREAD_CALLABLE_ROOT_BYTES)?;
        if existing == canonical::bytes(root).map_err(internal)? {
            verify_retained_closure(store, &root.authority)?;
            return Ok(());
        }
        return Err(ClewError::new(
            ErrorCode::BindingChanged,
            "callable root identifier is already bound to different evidence",
        ));
    }
    let objects = prepared
        .fact_shards
        .iter()
        .chain(&prepared.query_shards)
        .chain([&prepared.query_index_object, &prepared.evidence_object])
        .map(|object| (object.reference.object_schema.clone(), object.bytes.clone()))
        .collect::<Vec<_>>();
    let expected = prepared
        .fact_shards
        .iter()
        .chain(&prepared.query_shards)
        .chain([&prepared.query_index_object, &prepared.evidence_object])
        .map(|object| object.reference.clone())
        .collect::<Vec<_>>();
    let published = store.put_batch(objects)?;
    if published != expected {
        return Err(ClewError::new(
            ErrorCode::StateCorrupt,
            "CAS publication returned different callable object identities",
        ));
    }
    verify_retained_closure(store, &root.authority)?;
    write_json_create_new(state, &root_path, root)
}

/// Install a fully prepared callable fact set as loose CAS objects for focused
/// retained-service tests. Production publication remains pack-backed through
/// `publish_prepared`; this seam exists only so a test can remove one exact
/// object without corrupting an unrelated pack.
#[cfg(test)]
pub(crate) fn publish_prepared_loose_for_test(
    state: &StateAuthority,
    store: &CasStore,
    thread: &ThreadAuthority,
    prepared: &PreparedCallableFactSet,
) -> Result<ThreadCallableRoot, ClewError> {
    let root = ThreadCallableRoot {
        schema: THREAD_CALLABLE_ROOT_SCHEMA.into(),
        fact_set_id: prepared.projection.fact_set_id.clone(),
        thread_id: thread.thread_id.clone(),
        thread_authority_digest: thread.authority_digest.clone(),
        thread_context_id: prepared.authority.thread_context_id.clone(),
        thread_context_authority_digest: prepared.authority.thread_context_authority_digest.clone(),
        authority: prepared.authority.clone(),
        projection: prepared.projection.clone(),
    };
    validate_root(&root)?;
    validate_retained_closure_size(&root.authority)?;
    for object in prepared
        .fact_shards
        .iter()
        .chain(&prepared.query_shards)
        .chain([&prepared.query_index_object, &prepared.evidence_object])
    {
        if store.put(&object.reference.object_schema, &object.bytes)? != object.reference {
            return Err(corrupt(
                "CAS publication returned different callable object identity",
            ));
        }
    }
    verify_retained_closure(store, &root.authority)?;
    write_json_create_new(
        state,
        &callable_root_path(state, thread, &root.fact_set_id)?,
        &root,
    )?;
    Ok(root)
}

fn root_from_prepared(
    thread: &ThreadAuthority,
    context: &ThreadContextObject,
    prepared: &PreparedCallableFactSet,
) -> Result<ThreadCallableRoot, ClewError> {
    let root = ThreadCallableRoot {
        schema: THREAD_CALLABLE_ROOT_SCHEMA.into(),
        fact_set_id: prepared.projection.fact_set_id.clone(),
        thread_id: thread.thread_id.clone(),
        thread_authority_digest: thread.authority_digest.clone(),
        thread_context_id: context.context_id.clone(),
        thread_context_authority_digest: context.authority.authority_digest.clone(),
        authority: prepared.authority.clone(),
        projection: prepared.projection.clone(),
    };
    validate_root(&root)?;
    Ok(root)
}

fn validate_root(root: &ThreadCallableRoot) -> Result<(), ClewError> {
    crate::thread_callables::verify_authority_projection(&root.authority, &root.projection)?;
    if root.schema != THREAD_CALLABLE_ROOT_SCHEMA
        || root.authority.schema != CALLABLE_FACT_SET_SCHEMA
        || root.fact_set_id != format!("thread-callables:{}", root.authority.authority_digest)
        || root.thread_id != root.authority.thread_id
        || root.thread_authority_digest != root.authority.thread_authority_digest
        || root.thread_context_id != root.authority.thread_context_id
        || root.thread_context_authority_digest != root.authority.thread_context_authority_digest
        || root.projection.fact_set_id != root.fact_set_id
    {
        return Err(corrupt(
            "thread callable retained root authority is invalid",
        ));
    }
    Ok(())
}

fn load_prepared_from_root(
    store: &CasStore,
    root: &ThreadCallableRoot,
) -> Result<PreparedCallableFactSet, ClewError> {
    let evidence_object = read_prepared_object(
        store,
        &root.authority.evidence_ref,
        MAX_CALLABLE_EVIDENCE_OBJECT_BYTES,
    )?;
    let evidence: CallableFactSetEvidence = serde_json::from_slice(&evidence_object.bytes)
        .map_err(|_| corrupt("callable fact-set evidence is invalid"))?;
    let query_index_object = read_prepared_object(
        store,
        &root.authority.query_index_ref,
        MAX_CALLABLE_SHARD_BYTES,
    )?;
    let query_index: CallableQueryIndexManifest = serde_json::from_slice(&query_index_object.bytes)
        .map_err(|_| corrupt("callable query-index manifest is invalid"))?;
    let fact_shards = root
        .authority
        .fact_shards
        .iter()
        .map(|reference| read_prepared_object(store, &reference.object, MAX_CALLABLE_SHARD_BYTES))
        .collect::<Result<Vec<_>, _>>()?;
    let query_shards = query_index
        .shards
        .iter()
        .map(|reference| read_prepared_object(store, &reference.object, MAX_CALLABLE_SHARD_BYTES))
        .collect::<Result<Vec<_>, _>>()?;
    let prepared = PreparedCallableFactSet {
        authority: root.authority.clone(),
        evidence,
        projection: root.projection.clone(),
        fact_shards,
        query_shards,
        query_index,
        query_index_object,
        evidence_object,
        authority_bytes: canonical::bytes(&root.authority).map_err(internal)?,
        projection_bytes: canonical::bytes(&root.projection).map_err(internal)?,
    };
    crate::thread_callables::verify_prepared(&prepared)?;
    Ok(prepared)
}

fn read_prepared_object(
    store: &CasStore,
    reference: &CasObject,
    max_bytes: usize,
) -> Result<PreparedCasObject, ClewError> {
    let size = usize::try_from(reference.size)
        .map_err(|_| budget("callable CAS object exceeds host size"))?;
    if size > max_bytes {
        return Err(budget("callable CAS object exceeds its retained bound"));
    }
    let lease = store.read(reference, size)?;
    Ok(PreparedCasObject {
        reference: reference.clone(),
        bytes: lease.bytes().to_vec(),
    })
}

fn verify_retained_closure(
    store: &CasStore,
    authority: &CallableFactSetAuthority,
) -> Result<(), ClewError> {
    if !authority
        .direct_cas_closure
        .iter()
        .any(|object| object == &authority.evidence_ref)
        || !authority
            .direct_cas_closure
            .iter()
            .any(|object| object == &authority.query_index_ref)
    {
        return Err(corrupt("callable retained closure omits derived evidence"));
    }
    validate_retained_closure_size(authority)?;
    for object in &authority.direct_cas_closure {
        let size = usize::try_from(object.size)
            .map_err(|_| budget("retained CAS object exceeds host size"))?;
        store.read(object, size)?;
    }
    Ok(())
}

fn validate_retained_closure_size(authority: &CallableFactSetAuthority) -> Result<(), ClewError> {
    validate_direct_cas_closure_size(
        &authority.direct_cas_closure,
        CallableBudgets::frozen().max_direct_cas_closure_bytes,
    )?;
    Ok(())
}

fn revalidate_captured(
    state: &StateAuthority,
    thread_context: &ThreadContextObject,
    captured: &[CapturedMember],
) -> Result<(), ClewError> {
    for member in captured {
        let authority_path = state
            .session_root(&member.session.session_id)?
            .join("authority.json");
        if state.read_private_file(&authority_path, crate::session::MAX_PLAN_BYTES)?
            != canonical::bytes(&member.session).map_err(internal)?
            || canonical::bytes(&member.binding.session).map_err(internal)?
                != canonical::bytes(&member.session).map_err(internal)?
        {
            return Err(ClewError::new(
                ErrorCode::BindingChanged,
                "thread member session changed before callable publication",
            ));
        }
        let ready = load_session_generation(&member.session)?;
        if ready != member.ready {
            return Err(ClewError::new(
                ErrorCode::BindingChanged,
                "thread member generation changed before callable publication",
            ));
        }
        let binding = thread_context
            .authority
            .members
            .iter()
            .find(|binding| binding.member_alias == member.binding.member_alias)
            .ok_or_else(|| corrupt("thread context lost a selected member"))?;
        let context = member.session.load_context(&binding.context_id)?;
        if canonical::bytes(&context).map_err(internal)?
            != canonical::bytes(&member.context).map_err(internal)?
        {
            return Err(ClewError::new(
                ErrorCode::BindingChanged,
                "member context changed before callable publication",
            ));
        }
    }
    Ok(())
}

fn revalidate_persisted_thread(
    state: &StateAuthority,
    thread: &ThreadAuthority,
) -> Result<(), ClewError> {
    let path = state.thread_root(&thread.thread_id)?.join("authority.json");
    let bytes = state.read_private_file(&path, 1024 * 1024)?;
    if bytes != canonical::bytes(thread).map_err(internal)? {
        return Err(ClewError::new(
            ErrorCode::BindingChanged,
            "persisted thread authority changed before callable publication",
        ));
    }
    Ok(())
}

fn profile_digest() -> Result<String, ClewError> {
    canonical::hash(&json!({
        "schema":"codeclew-kotlin-descriptor-navigation-profile/1.0",
        "semanticAuthority":SEMANTIC_AUTHORITY,
        "extractorAuthority":EXTRACTOR_AUTHORITY,
        "factPayloadSchema":KOTLIN_SEMANTIC_FACT_SCHEMA,
        "acceptedPayloadSchemas":[
            "declaration-descriptor/0.1",
            "declaration-descriptor-boundary/0.1",
            "declaration-relation/0.1",
            "declaration-relation-boundary/0.1",
        ],
        "relationshipAuthority":"DECLARED_TOPOLOGY",
        "httpClaims":false,
    }))
    .map_err(internal)
}

fn adapter_digest(runtime: &RuntimeAuthority, compiler_version: &str) -> Result<String, ClewError> {
    let matches = runtime
        .workers
        .values()
        .filter(|worker| worker.compiler_version == compiler_version)
        .collect::<Vec<_>>();
    if matches.len() != 1 {
        return Err(ClewError::new(
            ErrorCode::UnsupportedKotlinVersion,
            "runtime does not contain one exact worker for the sealed compiler version",
        ));
    }
    kotlin_adapter_digest(&matches[0].tree_hash)
}

fn kotlin_fact_category(fact_key: &str) -> Result<Option<&str>, ClewError> {
    let value = fact_key
        .strip_prefix("kotlin:")
        .ok_or_else(|| corrupt("Kotlin semantic fact key has another namespace"))?;
    let (category, digest) = value
        .split_once(':')
        .ok_or_else(|| corrupt("Kotlin semantic fact key has no category digest"))?;
    if digest.len() != 64
        || !digest
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Err(corrupt("Kotlin semantic fact key digest is invalid"));
    }
    match category {
        // These facts remain retained by the sealed generation, but they are
        // outside the declaration/relation projection consumed by callables.
        // In particular, local CFG evidence must not affect callable graph
        // coverage or be parsed as a declaration/relation payload.
        "metadata" | "file" | "local-cfg" | "local-cfg-boundary" => Ok(None),
        "descriptor" | "descriptor-boundary" | "relation" | "relation-boundary" => {
            Ok(Some(category))
        }
        _ => Err(corrupt("Kotlin semantic fact category is unsupported")),
    }
}

fn category_matches_kind(category: &str, kind: KotlinSemanticPayloadKind) -> bool {
    matches!(
        (category, kind),
        (
            "descriptor",
            KotlinSemanticPayloadKind::DeclarationDescriptor
        ) | ("relation", KotlinSemanticPayloadKind::DeclarationRelation)
            | (
                "descriptor-boundary",
                KotlinSemanticPayloadKind::DeclarationDescriptorBoundary
            )
            | (
                "relation-boundary",
                KotlinSemanticPayloadKind::DeclarationRelationBoundary
            )
    )
}

fn context_cas(value: &Value, pointer: &str, object_schema: &str) -> Result<CasObject, ClewError> {
    context_cas_value(value.pointer(pointer), object_schema)
}

fn context_cas_value(value: Option<&Value>, object_schema: &str) -> Result<CasObject, ClewError> {
    let object: CasObject = serde_json::from_value(
        value
            .cloned()
            .ok_or_else(|| corrupt("context CAS authority is missing"))?,
    )
    .map_err(|_| corrupt("context CAS authority is invalid"))?;
    if object.schema != CAS_OBJECT_SCHEMA || object.object_schema != object_schema {
        return Err(corrupt("context CAS authority has another schema"));
    }
    Ok(object)
}

fn read_canonical_cas<T: for<'de> Deserialize<'de> + Serialize>(
    store: &CasStore,
    object: &CasObject,
) -> Result<T, ClewError> {
    let limit = usize::try_from(object.size).map_err(|_| budget("CAS object exceeds host size"))?;
    let lease = store.read(object, limit)?;
    let value = serde_json::from_slice(lease.bytes())
        .map_err(|_| corrupt("CAS payload is not valid JSON"))?;
    if canonical::bytes(&value).map_err(internal)? != lease.bytes() {
        return Err(corrupt("CAS JSON payload is not canonical"));
    }
    Ok(value)
}

fn callable_root_path(
    state: &StateAuthority,
    thread: &ThreadAuthority,
    fact_set_id: &str,
) -> Result<std::path::PathBuf, ClewError> {
    let digest = fact_set_id
        .strip_prefix("thread-callables:sha256:")
        .filter(|digest| {
            digest.len() == 64
                && digest
                    .bytes()
                    .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        })
        .ok_or_else(|| invalid("thread callable fact-set id is invalid"))?;
    let directory = state
        .thread_root(&thread.thread_id)?
        .join("callable-fact-sets");
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
        .map_err(|_| invalid("thread callable root escapes managed state"))?;
    let parent = relative
        .parent()
        .ok_or_else(|| invalid("thread callable root has no parent"))?;
    let name = relative
        .file_name()
        .ok_or_else(|| invalid("thread callable root has no file name"))?;
    let bytes = canonical::bytes(value).map_err(internal)?;
    let directory = state.directory(parent)?;
    if directory.atomic_create(name, &bytes)? {
        return Ok(());
    }
    if directory.read_file(name, MAX_THREAD_CALLABLE_ROOT_BYTES)? == bytes {
        Ok(())
    } else {
        Err(ClewError::new(
            ErrorCode::BindingChanged,
            "callable root identifier was concurrently bound to different evidence",
        ))
    }
}

fn validate_identifier(value: &str, label: &str) -> Result<(), ClewError> {
    if value.is_empty()
        || value.len() > 128
        || !crate::text_authority::is_nfc(value)
        || value.bytes().enumerate().any(|(index, byte)| {
            !(byte.is_ascii_alphanumeric()
                || (index > 0 && matches!(byte, b'-' | b'_' | b'.' | b':')))
        })
    {
        return Err(invalid(&format!("thread callable {label} is invalid")));
    }
    Ok(())
}

fn validate_relative_source(value: &str) -> Result<(), ClewError> {
    let path = Path::new(value);
    if value.is_empty()
        || value.len() > 4096
        || path.is_absolute()
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(corrupt("semantic source path is not repository-relative"));
    }
    Ok(())
}

fn checked_add_budget(
    left: usize,
    right: usize,
    limit: usize,
    message: &str,
) -> Result<usize, ClewError> {
    let value = left.checked_add(right).ok_or_else(|| budget(message))?;
    if value > limit {
        return Err(budget(message));
    }
    Ok(value)
}

fn invalid(message: &str) -> ClewError {
    ClewError::new(ErrorCode::InvalidInput, message)
}

fn corrupt(message: &str) -> ClewError {
    ClewError::new(ErrorCode::StateCorrupt, message)
}

fn budget(message: &str) -> ClewError {
    ClewError::new(ErrorCode::ResourceLimit, message)
}

fn internal(error: impl std::fmt::Display) -> ClewError {
    ClewError::new(ErrorCode::Internal, error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    const FACT_DIGEST: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    fn source_store() -> (tempfile::TempDir, CasStore) {
        let temporary = tempfile::tempdir().unwrap();
        let state = StateAuthority::open(temporary.path().join("v2")).unwrap();
        let store = CasStore::open(&state).unwrap();
        (temporary, store)
    }

    #[test]
    fn callable_category_routing_ignores_local_cfg_but_rejects_unknown_categories() {
        for category in ["metadata", "file", "local-cfg", "local-cfg-boundary"] {
            let fact_key = format!("kotlin:{category}:{FACT_DIGEST}");
            assert_eq!(kotlin_fact_category(&fact_key).unwrap(), None);
        }

        for category in [
            "descriptor",
            "descriptor-boundary",
            "relation",
            "relation-boundary",
        ] {
            let fact_key = format!("kotlin:{category}:{FACT_DIGEST}");
            assert_eq!(kotlin_fact_category(&fact_key).unwrap(), Some(category));
        }

        let error = kotlin_fact_category(&format!("kotlin:arbitrary:{FACT_DIGEST}"))
            .expect_err("unknown categories must remain fail-closed");
        assert_eq!(error.code, ErrorCode::StateCorrupt);
    }

    #[test]
    fn source_authority_is_read_once_and_every_range_is_checked() {
        let (_temporary, store) = source_store();
        let source = store
            .put(
                "codeclew-test-kotlin-source/1.0",
                "fun café() = 1\n".as_bytes(),
            )
            .unwrap();
        let sources = EffectiveSourceMap {
            index: BTreeMap::from([("src/Main.kt".into(), source.clone())]),
            worktree: BTreeMap::new(),
        };
        let mut verifier = SourceVerifier::default();
        for range in [(0, 3), (4, 9)] {
            assert_eq!(
                verifier
                    .bind(
                        &store,
                        &sources,
                        &json!({"file":"src/Main.kt","start":range.0,"end":range.1}),
                        KotlinSemanticPayloadKind::DeclarationDescriptor,
                    )
                    .unwrap(),
                Some(source.clone())
            );
        }
        assert_eq!(verifier.cached.len(), 1);
        assert_eq!(verifier.cached_bytes, source.size as usize);

        let error = verifier
            .bind(
                &store,
                &sources,
                &json!({"file":"src/Main.kt","start":7,"end":8}),
                KotlinSemanticPayloadKind::DeclarationDescriptor,
            )
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::StateCorrupt);
    }

    #[test]
    fn source_authority_rejects_missing_unsafe_and_non_utf8_inputs() {
        let (_temporary, store) = source_store();
        let binary = store
            .put("codeclew-test-kotlin-source/1.0", &[0xff, 0xfe])
            .unwrap();
        let sources = EffectiveSourceMap {
            index: BTreeMap::from([("src/Binary.kt".into(), binary)]),
            worktree: BTreeMap::new(),
        };
        let mut verifier = SourceVerifier::default();
        for payload in [
            json!({"file":"../Main.kt","start":0,"end":1}),
            json!({"file":"src/Missing.kt","start":0,"end":1}),
            json!({"file":"src/Binary.kt","start":0,"end":1}),
        ] {
            assert!(
                verifier
                    .bind(
                        &store,
                        &sources,
                        &payload,
                        KotlinSemanticPayloadKind::DeclarationDescriptor,
                    )
                    .is_err()
            );
        }
        assert!(
            verifier
                .bind(
                    &store,
                    &sources,
                    &json!({}),
                    KotlinSemanticPayloadKind::DeclarationRelationBoundary,
                )
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn effective_source_map_obeys_worktree_override_authority() {
        let index = CasObject::for_bytes("test/source/1", b"index").unwrap();
        let worktree = CasObject::for_bytes("test/source/1", b"worktree").unwrap();
        let map = EffectiveSourceMap {
            index: BTreeMap::from([
                ("regular.kt".into(), index.clone()),
                ("missing.kt".into(), index.clone()),
                ("symlink.kt".into(), index),
            ]),
            worktree: BTreeMap::from([
                (
                    "regular.kt".into(),
                    (WorktreeKind::Regular, Some(worktree.clone())),
                ),
                ("missing.kt".into(), (WorktreeKind::Missing, None)),
                ("symlink.kt".into(), (WorktreeKind::Symlink, None)),
            ]),
        };
        assert_eq!(map.get("regular.kt"), Some(&worktree));
        assert_eq!(map.get("missing.kt"), None);
        assert_eq!(map.get("symlink.kt"), None);
    }
}
