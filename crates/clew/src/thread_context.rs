use crate::canonical;
use crate::cas::{CAS_OBJECT_SCHEMA, CasObject, CasStore};
use crate::error::{ClewError, ErrorCode};
use crate::session::{ContextObject, SessionAuthority, validate_context_request};
use crate::state::StateAuthority;
use crate::thread::{ThreadAuthority, ThreadMemberBinding};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

pub const THREAD_CONTEXT_AUTHORITY_SCHEMA: &str = "codeclew-thread-context-authority/1.0";
pub const THREAD_CONTEXT_SCHEMA: &str = "codeclew-thread-context/1.0";
pub const THREAD_CONTEXT_EVIDENCE_SCHEMA: &str = "codeclew-thread-context-evidence/1.0";
pub const THREAD_CONTEXT_PROJECTION_SCHEMA: &str = "codeclew-thread-context-projection/1.0";
pub const THREAD_CONTEXT_RESULT_SCHEMA: &str = "codeclew-thread-context-result/1.0";

pub const MAX_THREAD_FACTS: usize = 4096;
pub const MAX_THREAD_EVIDENCE_BYTES: usize = 1024 * 1024;
pub const MAX_THREAD_SNIPPET_BYTES: usize = 16 * 1024;
pub const MAX_THREAD_SOURCE_WINDOWS: usize = 32;
pub const MAX_THREAD_SOURCE_BYTES: usize = 256 * 1024;
pub const MAX_THREAD_STDOUT_BYTES: usize = 64 * 1024;
const THREAD_PROJECTION_TARGET_BYTES: usize = 54 * 1024;
const MAX_THREAD_CONTEXT_OBJECT_BYTES: usize = 2 * 1024 * 1024;
const MEMBER_CONTEXT_EVIDENCE_SCHEMA: &str = "codeclew-context-evidence-object/3.0";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ThreadContextBudgets {
    pub query_max_roots: usize,
    pub max_facts: usize,
    pub max_evidence_bytes: usize,
    pub max_snippet_bytes: usize,
    pub max_source_windows: usize,
    pub max_source_bytes: usize,
    pub max_stdout_bytes: usize,
}

impl ThreadContextBudgets {
    fn new(query_max_roots: usize) -> Result<Self, ClewError> {
        if query_max_roots == 0 || query_max_roots > 256 {
            return Err(invalid(
                "thread context root limit must be between one and 256",
            ));
        }
        Ok(Self {
            query_max_roots,
            max_facts: MAX_THREAD_FACTS,
            max_evidence_bytes: MAX_THREAD_EVIDENCE_BYTES,
            max_snippet_bytes: MAX_THREAD_SNIPPET_BYTES,
            max_source_windows: MAX_THREAD_SOURCE_WINDOWS,
            max_source_bytes: MAX_THREAD_SOURCE_BYTES,
            max_stdout_bytes: MAX_THREAD_STDOUT_BYTES,
        })
    }

    fn validate(&self) -> Result<(), ClewError> {
        if *self != Self::new(self.query_max_roots)? {
            return Err(invalid("thread context budget authority is invalid"));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ThreadMemberContextBinding {
    pub member_alias: String,
    pub service_alias: String,
    pub session_id: String,
    pub session_authority_digest: String,
    pub repository_key: String,
    pub base_revision: String,
    pub language: String,
    pub compilations: Vec<String>,
    pub context_id: String,
    pub context_digest: String,
    pub evidence_digest: String,
    pub evidence_ref: CasObject,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ThreadContextAuthority {
    pub schema: String,
    pub authority_digest: String,
    pub binding_digest: String,
    pub thread_id: String,
    pub thread_authority_digest: String,
    pub intent: String,
    pub terms: Vec<String>,
    pub budgets: ThreadContextBudgets,
    pub members: Vec<ThreadMemberContextBinding>,
    pub evidence_digest: String,
    pub evidence_ref: CasObject,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ThreadContextObject {
    pub schema: String,
    pub context_id: String,
    pub authority: ThreadContextAuthority,
    pub evidence_digest: String,
    pub evidence_ref: CasObject,
    pub projection: Value,
    #[serde(skip)]
    pub evidence: Value,
}

#[derive(Debug, Clone)]
struct MemberContext {
    member: ThreadMemberBinding,
    context: ContextObject,
}

#[derive(Debug, Clone)]
struct SourceCandidate {
    identity: (String, String),
    compilations: Vec<String>,
    value: Value,
    windows: usize,
    bytes: usize,
}

/// Create one read-only, bounded composite over the current immutable member
/// sessions. Member contexts are independently durable; the composite is
/// published only after every fan-out succeeds and the thread is still OPEN.
pub fn create(
    thread: &ThreadAuthority,
    intent: &str,
    terms: &[String],
    max_roots: usize,
) -> Result<ThreadContextObject, ClewError> {
    validate_context_request(intent, terms)?;
    thread.verify()?;
    let budgets = ThreadContextBudgets::new(max_roots)?;
    let state = StateAuthority::process_default()?;
    thread.require_open_with_state(&state)?;

    let mut member_contexts = Vec::with_capacity(thread.members.len());
    for member in &thread.members {
        let (session, _) = SessionAuthority::load(&member.session.session_id)?;
        if canonical::bytes(&session).map_err(internal)?
            != canonical::bytes(&member.session).map_err(internal)?
        {
            return Err(invalid("thread member session authority changed"));
        }
        session.require_open()?;
        let (projection, evidence) =
            crate::context_v2::create(&session, intent, terms, max_roots, None)?;
        let context = session.store_context(
            None,
            intent.to_owned(),
            terms.to_vec(),
            projection,
            evidence,
        )?;
        member_contexts.push(MemberContext {
            member: member.clone(),
            context,
        });
    }

    let (authority, evidence, projection) =
        compose(thread, intent, terms, budgets, member_contexts)?;
    // This is the linearization point for thread context publication. The
    // short-lived guard rechecks OPEN after fan-out and excludes close/GC only
    // while the immutable CAS reference and root record are published.
    let _admission = thread.admit_with_state(&state)?;
    store_with_state(&state, thread, authority, evidence, projection)
}

impl ThreadContextObject {
    pub fn load(thread: &ThreadAuthority, context_id: &str) -> Result<Self, ClewError> {
        thread.verify()?;
        let state = StateAuthority::process_default()?;
        load_with_state(&state, thread, context_id)
    }
}

pub fn bounded_thread_context_stdout(context: &ThreadContextObject) -> Result<Value, ClewError> {
    let summary = json!({
        "schema":THREAD_CONTEXT_RESULT_SCHEMA,
        "threadId":context.authority.thread_id,
        "threadAuthorityDigest":context.authority.thread_authority_digest,
        "contextId":context.context_id,
        "contextAuthorityDigest":context.authority.authority_digest,
        "evidenceDigest":context.evidence_digest,
        "evidenceRef":context.evidence_ref,
        "context":context.projection,
    });
    let rendered = canonical::bytes(&summary).map_err(internal)?;
    validate_stdout_bytes(&rendered)?;
    Ok(summary)
}

fn validate_stdout_bytes(bytes: &[u8]) -> Result<(), ClewError> {
    // main emits canonical compact JSON followed by println!'s LF.
    if bytes.len().saturating_add(1) > MAX_THREAD_STDOUT_BYTES {
        return Err(ClewError::new(
            ErrorCode::SliceBudgetExceeded,
            "thread context stdout exceeds 64 KiB",
        ));
    }
    Ok(())
}

fn compose(
    thread: &ThreadAuthority,
    intent: &str,
    terms: &[String],
    budgets: ThreadContextBudgets,
    mut member_contexts: Vec<MemberContext>,
) -> Result<(ThreadContextAuthority, Value, Value), ClewError> {
    budgets.validate()?;
    validate_context_request(intent, terms)?;
    member_contexts.sort_by(|left, right| left.member.member_alias.cmp(&right.member.member_alias));
    if member_contexts.len() != thread.members.len() {
        return Err(invalid("thread context member set is incomplete"));
    }

    let mut normalized_terms = terms.to_vec();
    normalized_terms.sort();
    normalized_terms.dedup();
    let mut bindings = Vec::with_capacity(member_contexts.len());
    for (expected, actual) in thread.members.iter().zip(&member_contexts) {
        validate_member_context(expected, actual, intent, &normalized_terms)?;
        bindings.push(member_context_binding(actual)?);
    }
    let mut authority = ThreadContextAuthority {
        schema: THREAD_CONTEXT_AUTHORITY_SCHEMA.into(),
        authority_digest: String::new(),
        binding_digest: String::new(),
        thread_id: thread.thread_id.clone(),
        thread_authority_digest: thread.authority_digest.clone(),
        intent: intent.to_owned(),
        terms: normalized_terms,
        budgets,
        members: bindings,
        evidence_digest: String::new(),
        evidence_ref: CasObject {
            schema: CAS_OBJECT_SCHEMA.into(),
            object_schema: THREAD_CONTEXT_EVIDENCE_SCHEMA.into(),
            digest: format!("sha256:{}", "0".repeat(64)),
            size: 0,
        },
    };
    authority.binding_digest = binding_digest(&authority)?;

    let (matches, all_match_count) = select_matches(&member_contexts, authority.budgets.max_facts)?;
    let (sources, all_source_count, source_limited) = select_sources(
        &member_contexts,
        authority.budgets.max_source_windows,
        authority.budgets.max_source_bytes,
        authority.budgets.max_snippet_bytes,
    )?;
    let obligations = collect_obligations(&member_contexts)?;
    let member_completeness = collect_member_completeness(&member_contexts)?;
    let initially_truncated =
        matches.len() < all_match_count || sources.len() < all_source_count || source_limited;
    let (evidence, selected_matches, selected_sources) = bounded_evidence(
        thread,
        &authority,
        matches,
        sources,
        all_match_count,
        all_source_count,
        obligations,
        member_completeness,
        initially_truncated,
    )?;
    let evidence_bytes = canonical::bytes(&evidence).map_err(internal)?;
    authority.evidence_ref = CasObject::for_bytes(THREAD_CONTEXT_EVIDENCE_SCHEMA, &evidence_bytes)?;
    authority.evidence_digest = authority.evidence_ref.digest.clone();
    authority.authority_digest = authority_digest(&authority)?;
    validate_authority(thread, &authority)?;
    let context_id = format!("thread-context:{}", authority.authority_digest);
    let projection = bounded_projection(
        thread,
        &authority,
        &context_id,
        &evidence,
        &selected_matches,
        &selected_sources,
    )?;
    Ok((authority, evidence, projection))
}

fn validate_member_context(
    expected: &ThreadMemberBinding,
    actual: &MemberContext,
    intent: &str,
    terms: &[String],
) -> Result<(), ClewError> {
    let context = &actual.context;
    if actual.member.member_alias != expected.member_alias
        || canonical::bytes(&actual.member).map_err(internal)?
            != canonical::bytes(expected).map_err(internal)?
        || context.session_id != expected.session.session_id
        || context.session_authority_digest != expected.session.authority_digest
        || context.evidence_ref.schema != CAS_OBJECT_SCHEMA
        || context.evidence_ref.object_schema != MEMBER_CONTEXT_EVIDENCE_SCHEMA
        || context.evidence_ref.digest != context.evidence_digest
        || context.schema != crate::session::CONTEXT_SCHEMA
        || context.intent != intent
        || context.terms != terms
    {
        return Err(invalid("member context authority is invalid"));
    }
    let matches = context
        .evidence
        .pointer("/context/matches")
        .and_then(Value::as_array)
        .ok_or_else(|| invalid("member context has no selected facts"))?;
    if matches.iter().any(|fact| {
        fact.get("compilation")
            .and_then(Value::as_str)
            .is_none_or(|compilation| {
                !expected
                    .session
                    .compilations
                    .iter()
                    .any(|item| item == compilation)
            })
    }) {
        return Err(invalid("member fact compilation provenance is invalid"));
    }
    Ok(())
}

fn member_context_binding(
    member_context: &MemberContext,
) -> Result<ThreadMemberContextBinding, ClewError> {
    let session = &member_context.member.session;
    let context = &member_context.context;
    Ok(ThreadMemberContextBinding {
        member_alias: member_context.member.member_alias.clone(),
        service_alias: member_context.member.service_alias.clone(),
        session_id: session.session_id.clone(),
        session_authority_digest: session.authority_digest.clone(),
        repository_key: session.repository_key.clone(),
        base_revision: session.base_revision.clone(),
        language: session.language.uri().into(),
        compilations: session.compilations.clone(),
        context_id: context.context_id.clone(),
        context_digest: canonical::hash(context).map_err(internal)?,
        evidence_digest: context.evidence_digest.clone(),
        evidence_ref: context.evidence_ref.clone(),
    })
}

fn select_matches(
    contexts: &[MemberContext],
    limit: usize,
) -> Result<(Vec<Value>, usize), ClewError> {
    let mut lanes = BTreeMap::<(String, String), Vec<(&MemberContext, &Value)>>::new();
    let mut total = 0usize;
    for member_context in contexts {
        let member = &member_context.member;
        let context = &member_context.context;
        for fact in context
            .evidence
            .pointer("/context/matches")
            .and_then(Value::as_array)
            .ok_or_else(|| invalid("member context selected facts are invalid"))?
        {
            let compilation = fact
                .get("compilation")
                .and_then(Value::as_str)
                .ok_or_else(|| invalid("member fact has no compilation provenance"))?;
            lanes
                .entry((member.member_alias.clone(), compilation.to_owned()))
                .or_default()
                .push((member_context, fact));
            total = total.saturating_add(1);
        }
    }
    let lanes = lanes.into_values().collect::<Vec<_>>();
    let mut cursors = vec![0usize; lanes.len()];
    let mut provenances = BTreeMap::new();
    let mut selected = Vec::new();
    while selected.len() < limit {
        let mut progressed = false;
        for (lane, cursor) in lanes.iter().zip(&mut cursors) {
            if let Some((member_context, fact)) = lane.get(*cursor) {
                let provenance =
                    if let Some(value) = provenances.get(&member_context.member.member_alias) {
                        value
                    } else {
                        provenances.insert(
                            member_context.member.member_alias.clone(),
                            provenance(&member_context.member, &member_context.context)?,
                        );
                        provenances
                            .get(&member_context.member.member_alias)
                            .expect("inserted provenance")
                    };
                let mut wrapped = (*fact).clone();
                let object = wrapped
                    .as_object_mut()
                    .ok_or_else(|| invalid("member fact is not an object"))?;
                insert_provenance(object, provenance)?;
                selected.push(wrapped);
                *cursor += 1;
                progressed = true;
            }
            if selected.len() == limit {
                break;
            }
        }
        if !progressed {
            break;
        }
    }
    Ok((selected, total))
}

fn select_sources(
    contexts: &[MemberContext],
    max_windows: usize,
    max_bytes: usize,
    max_snippet_bytes: usize,
) -> Result<(Vec<Value>, usize, bool), ClewError> {
    let mut unique = BTreeMap::<(String, String), SourceCandidate>::new();
    for member_context in contexts {
        let member = &member_context.member;
        let context = &member_context.context;
        let provenance = provenance(member, context)?;
        let mut source_compilations = BTreeMap::<String, BTreeSet<String>>::new();
        for fact in context
            .evidence
            .pointer("/context/matches")
            .and_then(Value::as_array)
            .ok_or_else(|| invalid("member context selected facts are invalid"))?
        {
            let compilation = fact
                .get("compilation")
                .and_then(Value::as_str)
                .ok_or_else(|| invalid("member fact has no compilation provenance"))?;
            for path in crate::context_v2::fact_source_paths(&fact["payload"]) {
                source_compilations
                    .entry(path)
                    .or_default()
                    .insert(compilation.to_owned());
            }
        }
        for source in context
            .evidence
            .pointer("/context/sources")
            .and_then(Value::as_array)
            .ok_or_else(|| invalid("member context sources are invalid"))?
        {
            let file = source
                .get("fileId")
                .and_then(Value::as_str)
                .ok_or_else(|| invalid("member source has no file identity"))?;
            let compilations = source_compilations
                .get(file)
                .ok_or_else(|| invalid("member source has no exact compilation provenance"))?
                .iter()
                .cloned()
                .collect::<Vec<_>>();
            let (mut bounded, windows, bytes, truncated) =
                bounded_source(source, max_snippet_bytes)?;
            let object = bounded
                .as_object_mut()
                .ok_or_else(|| invalid("member source is not an object"))?;
            insert_provenance(object, &provenance)?;
            object.insert("compilations".into(), json!(compilations));
            object.insert("threadTruncated".into(), Value::Bool(truncated));
            let identity = (member.member_alias.clone(), file.to_owned());
            unique.insert(
                identity.clone(),
                SourceCandidate {
                    identity,
                    compilations,
                    value: bounded,
                    windows,
                    bytes,
                },
            );
        }
    }

    let total = unique.len();
    let mut lanes = BTreeMap::<(String, String), Vec<SourceCandidate>>::new();
    for source in unique.values() {
        for compilation in &source.compilations {
            lanes
                .entry((source.identity.0.clone(), compilation.clone()))
                .or_default()
                .push(source.clone());
        }
    }
    for lane in lanes.values_mut() {
        lane.sort_by(|left, right| left.identity.cmp(&right.identity));
    }
    let mut cursors = vec![0usize; lanes.len()];
    let lanes = lanes.into_values().collect::<Vec<_>>();
    let mut seen = BTreeSet::new();
    let mut selected = Vec::new();
    let mut windows = 0usize;
    let mut bytes = 0usize;
    let mut limited = unique
        .values()
        .any(|source| source.value.get("threadTruncated").and_then(Value::as_bool) == Some(true));
    loop {
        let mut progressed = false;
        for (lane, cursor) in lanes.iter().zip(&mut cursors) {
            while let Some(source) = lane.get(*cursor) {
                *cursor += 1;
                if !seen.insert(source.identity.clone()) {
                    continue;
                }
                progressed = true;
                if windows.saturating_add(source.windows) <= max_windows
                    && bytes.saturating_add(source.bytes) <= max_bytes
                {
                    windows += source.windows;
                    bytes += source.bytes;
                    selected.push(source.value.clone());
                } else {
                    limited = true;
                }
                break;
            }
        }
        if !progressed {
            break;
        }
    }
    Ok((selected, total, limited))
}

fn bounded_source(
    source: &Value,
    max_snippet_bytes: usize,
) -> Result<(Value, usize, usize, bool), ClewError> {
    let windows = source
        .get("windows")
        .and_then(Value::as_array)
        .ok_or_else(|| invalid("member source windows are invalid"))?;
    if windows.is_empty() {
        return Err(invalid("member source windows are empty"));
    }
    let mut selected = Vec::new();
    let mut bytes = 0usize;
    for window in windows {
        let text = window
            .get("text")
            .and_then(Value::as_str)
            .ok_or_else(|| invalid("member source window text is invalid"))?;
        if text.len() > max_snippet_bytes {
            return Err(invalid(
                "member source window exceeds the thread snippet limit",
            ));
        }
        if !selected.is_empty() && bytes.saturating_add(text.len()) > max_snippet_bytes {
            break;
        }
        bytes = bytes.saturating_add(text.len());
        selected.push(window.clone());
    }
    if selected.is_empty() {
        return Err(invalid("member source cannot fit the thread snippet limit"));
    }
    let start = selected[0]
        .get("startLine")
        .and_then(Value::as_u64)
        .ok_or_else(|| invalid("member source start line is invalid"))?;
    let end = selected
        .last()
        .and_then(|value| value.get("endLine"))
        .and_then(Value::as_u64)
        .ok_or_else(|| invalid("member source end line is invalid"))?;
    let text = selected
        .iter()
        .enumerate()
        .map(|(index, window)| {
            let value = window["text"].as_str().unwrap_or_default();
            if index == 0 {
                value.to_owned()
            } else {
                format!(
                    "CODECLEW_OMITTED_LINES_BEFORE_{}\n{}",
                    window["startLine"].as_u64().unwrap_or_default(),
                    value
                )
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    let mut bounded = source.clone();
    let object = bounded
        .as_object_mut()
        .ok_or_else(|| invalid("member source is not an object"))?;
    object.insert("startLine".into(), json!(start));
    object.insert("endLine".into(), json!(end));
    object.insert("text".into(), Value::String(text));
    object.insert("windows".into(), Value::Array(selected.clone()));
    let truncated = selected.len() != windows.len();
    if truncated {
        object.insert("completeFile".into(), Value::Bool(false));
    }
    Ok((bounded, selected.len(), bytes, truncated))
}

fn collect_obligations(contexts: &[MemberContext]) -> Result<Vec<Value>, ClewError> {
    let mut obligations = Vec::new();
    for member_context in contexts {
        for obligation in member_context
            .context
            .evidence
            .pointer("/context/verificationObligations")
            .and_then(Value::as_array)
            .ok_or_else(|| invalid("member context obligations are invalid"))?
        {
            obligations.push(json!({
                "memberAlias":member_context.member.member_alias,
                "serviceAlias":member_context.member.service_alias,
                "sessionId":member_context.member.session.session_id,
                "contextId":member_context.context.context_id,
                "obligation":obligation,
            }));
        }
    }
    Ok(obligations)
}

fn collect_member_completeness(contexts: &[MemberContext]) -> Result<Vec<Value>, ClewError> {
    contexts
        .iter()
        .map(|member_context| {
            let completeness = member_context
                .context
                .evidence
                .pointer("/context/completeness")
                .cloned()
                .ok_or_else(|| invalid("member context completeness is missing"))?;
            Ok(json!({
                "memberAlias":member_context.member.member_alias,
                "serviceAlias":member_context.member.service_alias,
                "sessionId":member_context.member.session.session_id,
                "contextId":member_context.context.context_id,
                "completeness":completeness,
            }))
        })
        .collect()
}

#[allow(clippy::too_many_arguments)]
fn bounded_evidence(
    thread: &ThreadAuthority,
    authority: &ThreadContextAuthority,
    matches: Vec<Value>,
    sources: Vec<Value>,
    all_match_count: usize,
    all_source_count: usize,
    obligations: Vec<Value>,
    member_completeness: Vec<Value>,
    initially_truncated: bool,
) -> Result<(Value, Vec<Value>, Vec<Value>), ClewError> {
    // Determine the immutable base cost once, then admit canonical item bytes
    // in one alternating pass. This keeps limit handling O(total bytes) rather
    // than repeatedly serializing an ever-shrinking multi-megabyte value.
    let empty = evidence_value(
        thread,
        authority,
        &[],
        &[],
        all_match_count,
        all_source_count,
        &obligations,
        &member_completeness,
        true,
    )?;
    let base_bytes = canonical::bytes(&empty).map_err(internal)?.len();
    if base_bytes > authority.budgets.max_evidence_bytes {
        return Err(ClewError::new(
            ErrorCode::SliceBudgetExceeded,
            "thread authority, member references, and obligations exceed 1 MiB",
        ));
    }
    // Reserve a small deterministic envelope for changed decimal counts and
    // structural commas. The final checked serialization remains authoritative.
    let mut remaining = authority
        .budgets
        .max_evidence_bytes
        .saturating_sub(base_bytes)
        .saturating_sub(8 * 1024);
    let mut selected_matches = Vec::new();
    let mut selected_sources = Vec::new();
    let mut match_index = 0usize;
    let mut source_index = 0usize;
    while match_index < matches.len() || source_index < sources.len() {
        if let Some(source) = sources.get(source_index) {
            let cost = canonical::bytes(source).map_err(internal)?.len() + 1;
            if cost <= remaining {
                selected_sources.push(source.clone());
                remaining -= cost;
            }
            source_index += 1;
        }
        if let Some(fact) = matches.get(match_index) {
            let cost = canonical::bytes(fact).map_err(internal)?.len() + 1;
            if cost <= remaining {
                selected_matches.push(fact.clone());
                remaining -= cost;
            }
            match_index += 1;
        }
    }
    let mut truncated = initially_truncated
        || selected_matches.len() < all_match_count
        || selected_sources.len() < all_source_count;
    let mut evidence = evidence_value(
        thread,
        authority,
        &selected_matches,
        &selected_sources,
        all_match_count,
        all_source_count,
        &obligations,
        &member_completeness,
        truncated,
    )?;
    while canonical::bytes(&evidence).map_err(internal)?.len()
        > authority.budgets.max_evidence_bytes
    {
        if selected_sources.len() >= selected_matches.len() && !selected_sources.is_empty() {
            selected_sources.pop();
        } else if !selected_matches.is_empty() {
            selected_matches.pop();
        } else {
            return Err(internal(
                "thread evidence budget accounting is inconsistent",
            ));
        }
        truncated = true;
        evidence = evidence_value(
            thread,
            authority,
            &selected_matches,
            &selected_sources,
            all_match_count,
            all_source_count,
            &obligations,
            &member_completeness,
            truncated,
        )?;
    }
    Ok((evidence, selected_matches, selected_sources))
}

#[allow(clippy::too_many_arguments)]
fn evidence_value(
    thread: &ThreadAuthority,
    authority: &ThreadContextAuthority,
    matches: &[Value],
    sources: &[Value],
    all_match_count: usize,
    all_source_count: usize,
    obligations: &[Value],
    member_completeness: &[Value],
    truncated: bool,
) -> Result<Value, ClewError> {
    let completeness = aggregate_completeness(member_completeness, truncated)?;
    Ok(json!({
        "schema":THREAD_CONTEXT_EVIDENCE_SCHEMA,
        "threadId":thread.thread_id,
        "threadAuthorityDigest":thread.authority_digest,
        "threadContextBindingDigest":authority.binding_digest,
        "task":{"intent":authority.intent,"terms":authority.terms},
        "budgets":authority.budgets,
        "members":authority.members,
        "matches":matches,
        "sources":sources,
        "memberCompleteness":member_completeness,
        "verificationObligations":obligations,
        "selection":{
            "availableFacts":all_match_count,
            "selectedFacts":matches.len(),
            "availableSources":all_source_count,
            "selectedSources":sources.len(),
            "truncated":truncated,
        },
        "completeness":completeness,
        "publicationPolicy":{
            "mode":"READ_ONLY",
            "status":"NOT_APPLICABLE",
            "automaticPublication":false,
        },
    }))
}

fn bounded_projection(
    thread: &ThreadAuthority,
    authority: &ThreadContextAuthority,
    context_id: &str,
    evidence: &Value,
    matches: &[Value],
    sources: &[Value],
) -> Result<Value, ClewError> {
    let obligations = evidence["verificationObligations"]
        .as_array()
        .ok_or_else(|| invalid("thread evidence obligations are invalid"))?;
    let mut projection = json!({
        "schema":THREAD_CONTEXT_PROJECTION_SCHEMA,
        "threadId":thread.thread_id,
        "threadAuthorityDigest":thread.authority_digest,
        "contextId":context_id,
        "contextAuthorityDigest":authority.authority_digest,
        "task":evidence["task"],
        "members":authority.members.iter().map(|member| json!({
            "memberAlias":member.member_alias,
            "serviceAlias":member.service_alias,
            "sessionId":member.session_id,
            "language":member.language,
            "compilations":member.compilations,
            "contextId":member.context_id,
            "contextDigest":member.context_digest,
            "evidenceDigest":member.evidence_digest,
        })).collect::<Vec<_>>(),
        "matches":[],
        "sources":[],
        "completeness":evidence["completeness"],
        "publicationPolicy":evidence["publicationPolicy"],
        "verificationObligations":[],
        "obligationCount":obligations.len(),
        "obligationsTruncated":false,
        "truncated":evidence.pointer("/selection/truncated").and_then(Value::as_bool).unwrap_or(false),
    });
    if canonical::bytes(&projection).map_err(internal)?.len() > THREAD_PROJECTION_TARGET_BYTES {
        return Err(ClewError::new(
            ErrorCode::SliceBudgetExceeded,
            "thread context authority exceeds the bounded projection limit",
        ));
    }
    for obligation in obligations {
        if !push_if_bounded(
            &mut projection,
            "verificationObligations",
            obligation,
            THREAD_PROJECTION_TARGET_BYTES,
        )? {
            projection["obligationsTruncated"] = Value::Bool(true);
            projection["truncated"] = Value::Bool(true);
            break;
        }
    }
    let mut match_index = 0usize;
    let mut source_index = 0usize;
    while match_index < matches.len() || source_index < sources.len() {
        if let Some(source) = sources.get(source_index) {
            if !push_if_bounded(
                &mut projection,
                "sources",
                source,
                THREAD_PROJECTION_TARGET_BYTES,
            )? {
                projection["truncated"] = Value::Bool(true);
            }
            source_index += 1;
        }
        if let Some(fact) = matches.get(match_index) {
            if !push_if_bounded(
                &mut projection,
                "matches",
                fact,
                THREAD_PROJECTION_TARGET_BYTES,
            )? {
                projection["truncated"] = Value::Bool(true);
            }
            match_index += 1;
        }
    }
    if canonical::bytes(&projection).map_err(internal)?.len() > THREAD_PROJECTION_TARGET_BYTES {
        return Err(internal("thread projection exceeded its checked bound"));
    }
    Ok(projection)
}

fn push_if_bounded(
    projection: &mut Value,
    key: &str,
    value: &Value,
    limit: usize,
) -> Result<bool, ClewError> {
    projection[key]
        .as_array_mut()
        .ok_or_else(|| internal("thread projection array is missing"))?
        .push(value.clone());
    if canonical::bytes(projection).map_err(internal)?.len() <= limit {
        return Ok(true);
    }
    projection[key]
        .as_array_mut()
        .ok_or_else(|| internal("thread projection array is missing"))?
        .pop();
    Ok(false)
}

fn aggregate_completeness(member_rows: &[Value], truncated: bool) -> Result<Value, ClewError> {
    let mut any_incomplete = false;
    let mut any_conditional = false;
    let mut all_verified = true;
    let mut all_supported = true;
    let mut all_query_complete = true;
    let mut unmatched_terms = BTreeSet::new();
    for row in member_rows {
        let completeness = row
            .get("completeness")
            .ok_or_else(|| invalid("member completeness row is invalid"))?;
        match completeness.get("status").and_then(Value::as_str) {
            Some("INCOMPLETE") => any_incomplete = true,
            Some("CONDITIONAL_TASK") => any_conditional = true,
            Some("COMPLETE_TASK") => {}
            _ => return Err(invalid("member completeness status is invalid")),
        }
        match completeness.get("certainty").and_then(Value::as_str) {
            Some("VERIFIED") => {}
            Some("UNSURE") => all_verified = false,
            _ => return Err(invalid("member completeness certainty is invalid")),
        }
        match completeness.get("support").and_then(Value::as_str) {
            Some("SUPPORTED") => {}
            Some("UNSUPPORTED") => all_supported = false,
            _ => return Err(invalid("member completeness support is invalid")),
        }
        match completeness.get("coverage").and_then(Value::as_str) {
            Some("QUERY_COMPLETE") => {}
            Some("PARTIAL") => all_query_complete = false,
            _ => return Err(invalid("member completeness coverage is invalid")),
        }
        for term in completeness
            .get("unmatchedTerms")
            .and_then(Value::as_array)
            .ok_or_else(|| invalid("member completeness unmatched terms are invalid"))?
        {
            unmatched_terms.insert(
                term.as_str()
                    .ok_or_else(|| invalid("member completeness unmatched term is invalid"))?
                    .to_owned(),
            );
        }
    }
    let status = if any_incomplete {
        "INCOMPLETE"
    } else if any_conditional || truncated || !all_supported || !all_query_complete || !all_verified
    {
        "CONDITIONAL_TASK"
    } else {
        "COMPLETE_TASK"
    };
    Ok(json!({
        "status":status,
        "support":if all_supported { "SUPPORTED" } else { "UNSUPPORTED" },
        "certainty":if all_verified && all_supported && all_query_complete && !truncated {
            "VERIFIED"
        } else {
            "UNSURE"
        },
        "coverage":if all_query_complete && !truncated { "QUERY_COMPLETE" } else { "PARTIAL" },
        "unmatchedTerms":unmatched_terms,
        "memberCount":member_rows.len(),
    }))
}

#[cfg(test)]
fn fair_values(lanes: Vec<Vec<Value>>, limit: usize) -> Vec<Value> {
    let mut cursors = vec![0usize; lanes.len()];
    let mut selected = Vec::new();
    while selected.len() < limit {
        let mut progressed = false;
        for (lane, cursor) in lanes.iter().zip(&mut cursors) {
            if let Some(value) = lane.get(*cursor) {
                selected.push(value.clone());
                *cursor += 1;
                progressed = true;
            }
            if selected.len() == limit {
                break;
            }
        }
        if !progressed {
            break;
        }
    }
    selected
}

fn provenance(
    member: &ThreadMemberBinding,
    context: &ContextObject,
) -> Result<BTreeMap<String, Value>, ClewError> {
    Ok(BTreeMap::from([
        ("memberAlias".into(), json!(member.member_alias)),
        ("serviceAlias".into(), json!(member.service_alias)),
        ("sessionId".into(), json!(member.session.session_id)),
        (
            "sessionAuthorityDigest".into(),
            json!(member.session.authority_digest),
        ),
        ("language".into(), json!(member.session.language.uri())),
        ("contextId".into(), json!(context.context_id)),
        (
            "contextDigest".into(),
            json!(canonical::hash(context).map_err(internal)?),
        ),
        ("evidenceDigest".into(), json!(context.evidence_digest)),
    ]))
}

fn insert_provenance(
    object: &mut serde_json::Map<String, Value>,
    provenance: &BTreeMap<String, Value>,
) -> Result<(), ClewError> {
    if provenance.keys().any(|key| object.contains_key(key)) {
        return Err(invalid(
            "member evidence collides with reserved thread provenance",
        ));
    }
    object.extend(
        provenance
            .iter()
            .map(|(key, value)| (key.clone(), value.clone())),
    );
    Ok(())
}

fn authority_digest(authority: &ThreadContextAuthority) -> Result<String, ClewError> {
    let mut unsigned = authority.clone();
    unsigned.authority_digest.clear();
    canonical::hash(&unsigned).map_err(internal)
}

fn binding_digest(authority: &ThreadContextAuthority) -> Result<String, ClewError> {
    canonical::hash(&json!({
        "schema":"codeclew-thread-context-binding/1.0",
        "threadId":authority.thread_id,
        "threadAuthorityDigest":authority.thread_authority_digest,
        "intent":authority.intent,
        "terms":authority.terms,
        "budgets":authority.budgets,
        "members":authority.members,
    }))
    .map_err(internal)
}

fn validate_authority(
    thread: &ThreadAuthority,
    authority: &ThreadContextAuthority,
) -> Result<(), ClewError> {
    authority.budgets.validate()?;
    validate_context_request(&authority.intent, &authority.terms)?;
    if authority.schema != THREAD_CONTEXT_AUTHORITY_SCHEMA
        || authority.thread_id != thread.thread_id
        || authority.thread_authority_digest != thread.authority_digest
        || authority.members.len() != thread.members.len()
        || authority.binding_digest != binding_digest(authority)?
        || authority.authority_digest != authority_digest(authority)?
        || authority.terms.windows(2).any(|pair| pair[0] >= pair[1])
        || authority.evidence_ref.schema != CAS_OBJECT_SCHEMA
        || authority.evidence_ref.object_schema != THREAD_CONTEXT_EVIDENCE_SCHEMA
        || authority.evidence_ref.digest != authority.evidence_digest
        || !sha256_digest(&authority.binding_digest)
        || !sha256_digest(&authority.evidence_digest)
        || usize::try_from(authority.evidence_ref.size)
            .map_or(true, |size| size > authority.budgets.max_evidence_bytes)
    {
        return Err(invalid("thread context authority is invalid"));
    }
    for (member, binding) in thread.members.iter().zip(&authority.members) {
        if binding.member_alias != member.member_alias
            || binding.service_alias != member.service_alias
            || binding.session_id != member.session.session_id
            || binding.session_authority_digest != member.session.authority_digest
            || binding.repository_key != member.session.repository_key
            || binding.base_revision != member.session.base_revision
            || binding.language != member.session.language.uri()
            || binding.compilations != member.session.compilations
            || !content_id(&binding.context_id, "context:")
            || !sha256_digest(&binding.context_digest)
            || binding.evidence_ref.schema != CAS_OBJECT_SCHEMA
            || binding.evidence_ref.object_schema != MEMBER_CONTEXT_EVIDENCE_SCHEMA
            || binding.evidence_ref.digest != binding.evidence_digest
        {
            return Err(invalid("thread context member binding is invalid"));
        }
    }
    Ok(())
}

fn store_with_state(
    state: &StateAuthority,
    thread: &ThreadAuthority,
    authority: ThreadContextAuthority,
    evidence: Value,
    projection: Value,
) -> Result<ThreadContextObject, ClewError> {
    thread.verify()?;
    validate_authority(thread, &authority)?;
    let context_id = format!("thread-context:{}", authority.authority_digest);
    let evidence_bytes = canonical::bytes(&evidence).map_err(internal)?;
    validate_evidence_bytes(&evidence_bytes)?;
    let expected_evidence_ref =
        CasObject::for_bytes(THREAD_CONTEXT_EVIDENCE_SCHEMA, &evidence_bytes)?;
    if expected_evidence_ref != authority.evidence_ref {
        return Err(invalid(
            "thread context evidence differs from its bound authority",
        ));
    }
    validate_payload(thread, &authority, &context_id, &projection, &evidence)?;
    let mut object = ThreadContextObject {
        schema: THREAD_CONTEXT_SCHEMA.into(),
        context_id: context_id.clone(),
        evidence_digest: authority.evidence_digest.clone(),
        evidence_ref: authority.evidence_ref.clone(),
        authority,
        projection,
        evidence,
    };
    // A result that cannot be returned must not acquire a durable composite
    // root. The full CLI envelope is therefore checked before the CAS write
    // and root-record publication linearization point.
    bounded_thread_context_stdout(&object)?;
    let store = CasStore::open(state)?;
    let evidence_ref = store.put(THREAD_CONTEXT_EVIDENCE_SCHEMA, &evidence_bytes)?;
    if evidence_ref != object.authority.evidence_ref {
        return Err(internal(
            "CAS returned a different thread evidence identity",
        ));
    }
    object.evidence_ref = evidence_ref.clone();
    object.evidence_digest = evidence_ref.digest;
    let root = state.thread_root(&thread.thread_id)?;
    state.write_private_atomic(
        &root.join("contexts").join(id_filename(&context_id)?),
        &canonical::bytes(&object).map_err(internal)?,
    )?;
    Ok(object)
}

fn load_with_state(
    state: &StateAuthority,
    thread: &ThreadAuthority,
    context_id: &str,
) -> Result<ThreadContextObject, ClewError> {
    let root = state.thread_root(&thread.thread_id)?;
    let bytes = state
        .read_private_file(
            &root.join("contexts").join(id_filename(context_id)?),
            MAX_THREAD_CONTEXT_OBJECT_BYTES,
        )
        .map_err(|_| invalid("thread context is missing or exceeds its object limit"))?;
    let mut object: ThreadContextObject =
        serde_json::from_slice(&bytes).map_err(|_| invalid("thread context object is invalid"))?;
    if canonical::bytes(&object).map_err(internal)? != bytes
        || object.schema != THREAD_CONTEXT_SCHEMA
        || object.context_id != context_id
        || object.context_id != format!("thread-context:{}", object.authority.authority_digest)
        || object.evidence_ref.schema != CAS_OBJECT_SCHEMA
        || object.evidence_ref.object_schema != THREAD_CONTEXT_EVIDENCE_SCHEMA
        || object.evidence_ref.digest != object.evidence_digest
        || object.evidence_ref != object.authority.evidence_ref
        || object.evidence_digest != object.authority.evidence_digest
    {
        return Err(invalid("thread context object authority is invalid"));
    }
    validate_authority(thread, &object.authority)?;
    let store = CasStore::open(state)?;
    let lease = store.read(&object.evidence_ref, MAX_THREAD_EVIDENCE_BYTES)?;
    validate_evidence_bytes(lease.bytes())?;
    object.evidence = serde_json::from_slice(lease.bytes())
        .map_err(|_| invalid("thread context evidence is invalid"))?;
    if canonical::bytes(&object.evidence).map_err(internal)? != lease.bytes() {
        return Err(invalid("thread context evidence is not canonical"));
    }
    validate_payload(
        thread,
        &object.authority,
        &object.context_id,
        &object.projection,
        &object.evidence,
    )?;
    Ok(object)
}

fn validate_payload(
    thread: &ThreadAuthority,
    authority: &ThreadContextAuthority,
    context_id: &str,
    projection: &Value,
    evidence: &Value,
) -> Result<(), ClewError> {
    let projected_members = authority
        .members
        .iter()
        .map(|member| {
            json!({
                "memberAlias":member.member_alias,
                "serviceAlias":member.service_alias,
                "sessionId":member.session_id,
                "language":member.language,
                "compilations":member.compilations,
                "contextId":member.context_id,
                "contextDigest":member.context_digest,
                "evidenceDigest":member.evidence_digest,
            })
        })
        .collect::<Vec<_>>();
    if evidence.get("schema").and_then(Value::as_str) != Some(THREAD_CONTEXT_EVIDENCE_SCHEMA)
        || evidence.get("threadId").and_then(Value::as_str) != Some(thread.thread_id.as_str())
        || evidence
            .get("threadAuthorityDigest")
            .and_then(Value::as_str)
            != Some(thread.authority_digest.as_str())
        || evidence
            .get("threadContextBindingDigest")
            .and_then(Value::as_str)
            != Some(authority.binding_digest.as_str())
        || evidence.get("members") != Some(&json!(authority.members))
        || evidence.get("budgets") != Some(&json!(authority.budgets))
        || evidence.pointer("/task/intent").and_then(Value::as_str)
            != Some(authority.intent.as_str())
        || evidence.pointer("/task/terms") != Some(&json!(authority.terms))
        || projection.get("schema").and_then(Value::as_str)
            != Some(THREAD_CONTEXT_PROJECTION_SCHEMA)
        || projection.get("threadId").and_then(Value::as_str) != Some(thread.thread_id.as_str())
        || projection.get("contextId").and_then(Value::as_str) != Some(context_id)
        || projection
            .get("threadAuthorityDigest")
            .and_then(Value::as_str)
            != Some(thread.authority_digest.as_str())
        || projection
            .get("contextAuthorityDigest")
            .and_then(Value::as_str)
            != Some(authority.authority_digest.as_str())
        || projection.get("task") != evidence.get("task")
        || projection.get("members") != Some(&json!(projected_members))
        || projection.get("completeness") != evidence.get("completeness")
        || projection.get("publicationPolicy") != evidence.get("publicationPolicy")
        || canonical::bytes(projection).map_err(internal)?.len() > THREAD_PROJECTION_TARGET_BYTES
    {
        return Err(invalid("thread context payload authority is invalid"));
    }
    let evidence_bytes = canonical::bytes(evidence).map_err(internal)?;
    validate_evidence_bytes(&evidence_bytes)?;
    if CasObject::for_bytes(THREAD_CONTEXT_EVIDENCE_SCHEMA, &evidence_bytes)?
        != authority.evidence_ref
    {
        return Err(invalid("thread context evidence binding is invalid"));
    }
    let matches = evidence
        .get("matches")
        .and_then(Value::as_array)
        .ok_or_else(|| invalid("thread context facts are invalid"))?;
    if matches.len() > authority.budgets.max_facts {
        return Err(invalid("thread context fact budget is invalid"));
    }
    let selection = evidence
        .get("selection")
        .and_then(Value::as_object)
        .ok_or_else(|| invalid("thread selection authority is invalid"))?;
    let available_facts = usize_field(selection, "availableFacts")?;
    let selected_facts = usize_field(selection, "selectedFacts")?;
    let available_sources = usize_field(selection, "availableSources")?;
    let selected_sources = usize_field(selection, "selectedSources")?;
    if selected_facts != matches.len() || selected_facts > available_facts {
        return Err(invalid("thread fact selection authority is invalid"));
    }
    let known = authority
        .members
        .iter()
        .map(|member| (member.member_alias.as_str(), member))
        .collect::<BTreeMap<_, _>>();
    for fact in matches {
        let alias = fact
            .get("memberAlias")
            .and_then(Value::as_str)
            .ok_or_else(|| invalid("thread fact has no member provenance"))?;
        let compilation = fact
            .get("compilation")
            .and_then(Value::as_str)
            .ok_or_else(|| invalid("thread fact has no compilation provenance"))?;
        if known
            .get(alias)
            .is_none_or(|member| !member.compilations.iter().any(|item| item == compilation))
        {
            return Err(invalid("thread fact provenance is invalid"));
        }
    }
    let mut window_count = 0usize;
    let mut source_bytes = 0usize;
    for source in evidence
        .get("sources")
        .and_then(Value::as_array)
        .ok_or_else(|| invalid("thread context sources are invalid"))?
    {
        let alias = source
            .get("memberAlias")
            .and_then(Value::as_str)
            .ok_or_else(|| invalid("thread source has no member provenance"))?;
        let compilations = source
            .get("compilations")
            .and_then(Value::as_array)
            .ok_or_else(|| invalid("thread source has no compilation provenance"))?;
        let windows = source
            .get("windows")
            .and_then(Value::as_array)
            .ok_or_else(|| invalid("thread source windows are invalid"))?;
        let member = known
            .get(alias)
            .ok_or_else(|| invalid("thread source member provenance is invalid"))?;
        let compilation_names = compilations
            .iter()
            .map(|value| {
                value
                    .as_str()
                    .map(str::to_owned)
                    .ok_or_else(|| invalid("thread source compilation is invalid"))
            })
            .collect::<Result<Vec<_>, _>>()?;
        if compilations.is_empty()
            || compilation_names.windows(2).any(|pair| pair[0] >= pair[1])
            || compilations.iter().any(|value| {
                value.as_str().is_none_or(|compilation| {
                    !member.compilations.iter().any(|item| item == compilation)
                })
            })
            || windows.is_empty()
        {
            return Err(invalid("thread source compilation provenance is invalid"));
        }
        window_count = window_count.saturating_add(windows.len());
        for window in windows {
            let text = window
                .get("text")
                .and_then(Value::as_str)
                .ok_or_else(|| invalid("thread source window text is invalid"))?;
            if text.len() > authority.budgets.max_snippet_bytes {
                return Err(invalid("thread source snippet budget is invalid"));
            }
            source_bytes = source_bytes.saturating_add(text.len());
        }
    }
    validate_source_budget(&authority.budgets, window_count, source_bytes)?;
    let sources = evidence["sources"]
        .as_array()
        .ok_or_else(|| invalid("thread context sources are invalid"))?;
    if selected_sources != sources.len() || selected_sources > available_sources {
        return Err(invalid("thread source selection authority is invalid"));
    }
    let member_completeness = evidence
        .get("memberCompleteness")
        .and_then(Value::as_array)
        .ok_or_else(|| invalid("thread member completeness is invalid"))?;
    if member_completeness.len() != authority.members.len() {
        return Err(invalid("thread member completeness set is incomplete"));
    }
    for (binding, row) in authority.members.iter().zip(member_completeness) {
        if row.get("memberAlias").and_then(Value::as_str) != Some(binding.member_alias.as_str())
            || row.get("serviceAlias").and_then(Value::as_str)
                != Some(binding.service_alias.as_str())
            || row.get("sessionId").and_then(Value::as_str) != Some(binding.session_id.as_str())
            || row.get("contextId").and_then(Value::as_str) != Some(binding.context_id.as_str())
        {
            return Err(invalid("thread member completeness provenance is invalid"));
        }
    }
    for row in evidence
        .get("verificationObligations")
        .and_then(Value::as_array)
        .ok_or_else(|| invalid("thread verification obligations are invalid"))?
    {
        let alias = row
            .get("memberAlias")
            .and_then(Value::as_str)
            .ok_or_else(|| invalid("thread obligation has no member provenance"))?;
        let Some(binding) = known.get(alias) else {
            return Err(invalid("thread obligation member provenance is invalid"));
        };
        if row.get("serviceAlias").and_then(Value::as_str) != Some(binding.service_alias.as_str())
            || row.get("sessionId").and_then(Value::as_str) != Some(binding.session_id.as_str())
            || row.get("contextId").and_then(Value::as_str) != Some(binding.context_id.as_str())
            || row.get("obligation").is_none()
        {
            return Err(invalid("thread obligation provenance is invalid"));
        }
    }
    let truncated = evidence
        .pointer("/selection/truncated")
        .and_then(Value::as_bool)
        .ok_or_else(|| invalid("thread selection authority is invalid"))?;
    let necessarily_truncated = selected_facts < available_facts
        || selected_sources < available_sources
        || sources
            .iter()
            .any(|source| source.get("threadTruncated").and_then(Value::as_bool) == Some(true));
    if necessarily_truncated && !truncated {
        return Err(invalid("thread selection truncation authority is invalid"));
    }
    let projected_truncated = projection
        .get("truncated")
        .and_then(Value::as_bool)
        .ok_or_else(|| invalid("thread projection truncation authority is invalid"))?;
    if truncated && !projected_truncated {
        return Err(invalid("thread projection hides evidence truncation"));
    }
    let obligations = evidence["verificationObligations"]
        .as_array()
        .ok_or_else(|| invalid("thread verification obligations are invalid"))?;
    if projection.get("obligationCount").and_then(Value::as_u64) != Some(obligations.len() as u64) {
        return Err(invalid("thread projection obligation count is invalid"));
    }
    if evidence.get("completeness")
        != Some(&aggregate_completeness(member_completeness, truncated)?)
        || evidence
            .pointer("/publicationPolicy/mode")
            .and_then(Value::as_str)
            != Some("READ_ONLY")
    {
        return Err(invalid(
            "thread completeness or publication policy is invalid",
        ));
    }
    let expected_projection =
        bounded_projection(thread, authority, context_id, evidence, matches, sources)?;
    if canonical::bytes(projection).map_err(internal)?
        != canonical::bytes(&expected_projection).map_err(internal)?
    {
        return Err(invalid(
            "thread projection differs from deterministic evidence projection",
        ));
    }
    Ok(())
}

fn validate_evidence_bytes(bytes: &[u8]) -> Result<(), ClewError> {
    if bytes.len() > MAX_THREAD_EVIDENCE_BYTES {
        return Err(ClewError::new(
            ErrorCode::SliceBudgetExceeded,
            "thread context evidence exceeds 1 MiB",
        ));
    }
    Ok(())
}

fn validate_source_budget(
    budgets: &ThreadContextBudgets,
    window_count: usize,
    source_bytes: usize,
) -> Result<(), ClewError> {
    if window_count > budgets.max_source_windows || source_bytes > budgets.max_source_bytes {
        return Err(invalid("thread source global budget is invalid"));
    }
    Ok(())
}

fn usize_field(object: &serde_json::Map<String, Value>, key: &str) -> Result<usize, ClewError> {
    let value = object
        .get(key)
        .and_then(Value::as_u64)
        .ok_or_else(|| invalid("thread selection count is invalid"))?;
    usize::try_from(value).map_err(|_| invalid("thread selection count exceeds host size"))
}

fn id_filename(value: &str) -> Result<String, ClewError> {
    let (_, digest) = value
        .rsplit_once("sha256:")
        .ok_or_else(|| invalid("thread context identifier has no digest"))?;
    if digest.len() != 64
        || !digest
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Err(invalid("thread context identifier digest is invalid"));
    }
    Ok(format!("{digest}.json"))
}

fn content_id(value: &str, prefix: &str) -> bool {
    value.strip_prefix(prefix).is_some_and(sha256_digest)
}

fn sha256_digest(value: &str) -> bool {
    value.len() == 71
        && value.starts_with("sha256:")
        && value[7..]
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

fn invalid(message: &str) -> ClewError {
    ClewError::new(ErrorCode::InvalidInput, message)
}

fn internal(error: impl std::fmt::Display) -> ClewError {
    ClewError::new(ErrorCode::Internal, error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::RuntimeMode;
    use crate::session::{CONTEXT_SCHEMA, ModelCachePolicy, SESSION_SCHEMA, SessionLanguage};
    use crate::thread::{ThreadMemberBinding, create_with_state};

    fn digest(seed: char) -> String {
        format!(
            "sha256:{}",
            std::iter::repeat_n(seed, 64).collect::<String>()
        )
    }

    fn session(seed: char, repository: &str, language: SessionLanguage) -> SessionAuthority {
        let mut session = SessionAuthority {
            schema: SESSION_SCHEMA.into(),
            authority_digest: String::new(),
            session_id: format!(
                "session:{}",
                std::iter::repeat_n(seed, 64).collect::<String>()
            ),
            repository_key: repository.into(),
            base_revision: std::iter::repeat_n(seed, 40).collect(),
            target_ref: "refs/heads/main".into(),
            target_oid: std::iter::repeat_n(seed, 40).collect(),
            runtime_key: format!(
                "runtime:{}",
                std::iter::repeat_n(seed, 64).collect::<String>()
            ),
            runtime_mode: RuntimeMode::Development,
            language,
            compilations: vec![":/main".into()],
            generation_jobs: None,
            model_cache_policy: ModelCachePolicy::NonCacheable,
            model_cache_authority: None,
            created_unix_ms: 1,
        };
        let mut unsigned = session.clone();
        unsigned.authority_digest.clear();
        session.authority_digest = canonical::hash(&unsigned).unwrap();
        session
    }

    fn member(alias: &str, session: SessionAuthority) -> ThreadMemberBinding {
        ThreadMemberBinding {
            member_alias: alias.into(),
            service_alias: format!("{alias}-service"),
            session,
        }
    }

    fn context(member: &ThreadMemberBinding, seed: char, status: &str) -> MemberContext {
        let evidence_ref = CasObject {
            schema: CAS_OBJECT_SCHEMA.into(),
            object_schema: MEMBER_CONTEXT_EVIDENCE_SCHEMA.into(),
            digest: digest(seed),
            size: 1,
        };
        let file = "src/main.py";
        let evidence = json!({
            "context":{
                "matches":[{
                    "compilation":":/main",
                    "factKey":"same-fact-key",
                    "domainUri":"language:test",
                    "payloadRef":evidence_ref,
                    "payload":{"file":file,"name":"shared"},
                }],
                "sources":[{
                    "fileId":file,
                    "contentRef":evidence_ref,
                    "startLine":1,
                    "endLine":1,
                    "text":"shared = 1",
                    "windows":[{"startLine":1,"endLine":1,"text":"shared = 1"}],
                    "completeFile":true,
                }],
                "completeness":{
                    "status":status,
                    "support":"SUPPORTED",
                    "coverage":if status == "COMPLETE_TASK" { "QUERY_COMPLETE" } else { "PARTIAL" },
                    "certainty":if status == "COMPLETE_TASK" { "VERIFIED" } else { "UNSURE" },
                    "unmatchedTerms":if status == "COMPLETE_TASK" { json!([]) } else { json!(["shared"]) },
                },
                "verificationObligations":if status == "COMPLETE_TASK" { json!([]) } else { json!([{"id":"verify"}]) },
            }
        });
        let context = ContextObject {
            schema: CONTEXT_SCHEMA.into(),
            context_id: format!("context:{}", digest(seed)),
            session_id: member.session.session_id.clone(),
            session_authority_digest: member.session.authority_digest.clone(),
            parent_context_id: None,
            intent: "trace shared contract".into(),
            terms: vec!["shared".into()],
            evidence_digest: evidence_ref.digest.clone(),
            evidence_ref,
            projection: json!({"fixture":true}),
            evidence,
        };
        MemberContext {
            member: member.clone(),
            context,
        }
    }

    fn fixture() -> (
        tempfile::TempDir,
        StateAuthority,
        ThreadAuthority,
        Vec<MemberContext>,
    ) {
        let temporary = tempfile::tempdir().unwrap();
        let state = StateAuthority::open(temporary.path().join("v2")).unwrap();
        let left = member("left", session('a', "repo:left", SessionLanguage::Python));
        let right = member("right", session('b', "repo:right", SessionLanguage::Python));
        let thread = create_with_state(
            &state,
            "thread:fixed".into(),
            1,
            vec![right.clone(), left.clone()],
        )
        .unwrap();
        let contexts = vec![
            context(&right, 'd', "CONDITIONAL_TASK"),
            context(&left, 'c', "COMPLETE_TASK"),
        ];
        (temporary, state, thread, contexts)
    }

    fn context_with_source_lengths(template: &MemberContext, lengths: &[usize]) -> MemberContext {
        let mut context = template.clone();
        let mut facts = Vec::with_capacity(lengths.len());
        let mut sources = Vec::with_capacity(lengths.len());
        for (index, length) in lengths.iter().copied().enumerate() {
            let file = format!("src/exact-{index:03}.py");
            facts.push(json!({
                "compilation":":/main",
                "factKey":format!("exact-{index:03}"),
                "domainUri":"language:test",
                "payload":{"file":file},
            }));
            let text = "x".repeat(length);
            sources.push(json!({
                "fileId":file,
                "contentRef":context.context.evidence_ref,
                "startLine":1,
                "endLine":1,
                "text":text,
                "windows":[{"startLine":1,"endLine":1,"text":text}],
                "completeFile":true,
            }));
        }
        context.context.evidence["context"]["matches"] = Value::Array(facts);
        context.context.evidence["context"]["sources"] = Value::Array(sources);
        context
    }

    fn selected_source_usage(sources: &[Value]) -> (usize, usize) {
        sources.iter().fold((0, 0), |(windows, bytes), source| {
            let rows = source["windows"].as_array().unwrap();
            (
                windows + rows.len(),
                bytes
                    + rows
                        .iter()
                        .map(|window| window["text"].as_str().unwrap().len())
                        .sum::<usize>(),
            )
        })
    }

    #[test]
    fn thread_context_is_order_independent_and_namespaces_collisions() {
        let (_temporary, _state, thread, contexts) = fixture();
        let forward = compose(
            &thread,
            "trace shared contract",
            &["shared".into()],
            ThreadContextBudgets::new(2).unwrap(),
            contexts.clone(),
        )
        .unwrap();
        let mut reversed = contexts;
        reversed.reverse();
        let reverse = compose(
            &thread,
            "trace shared contract",
            &["shared".into()],
            ThreadContextBudgets::new(2).unwrap(),
            reversed,
        )
        .unwrap();
        assert_eq!(forward.0.authority_digest, reverse.0.authority_digest);
        assert_eq!(
            canonical::bytes(&forward.1).unwrap(),
            canonical::bytes(&reverse.1).unwrap()
        );
        let aliases = forward
            .1
            .get("matches")
            .unwrap()
            .as_array()
            .unwrap()
            .iter()
            .map(|value| value["memberAlias"].as_str().unwrap())
            .collect::<BTreeSet<_>>();
        assert_eq!(aliases, BTreeSet::from(["left", "right"]));
    }

    #[test]
    fn context_change_does_not_change_thread_authority_or_upgrade_certainty() {
        let (_temporary, _state, thread, mut contexts) = fixture();
        let thread_digest = thread.authority_digest.clone();
        let first = compose(
            &thread,
            "trace shared contract",
            &["shared".into()],
            ThreadContextBudgets::new(2).unwrap(),
            contexts.clone(),
        )
        .unwrap();
        contexts[0].context.context_id = format!("context:{}", digest('e'));
        let second = compose(
            &thread,
            "trace shared contract",
            &["shared".into()],
            ThreadContextBudgets::new(2).unwrap(),
            contexts,
        )
        .unwrap();
        assert_ne!(first.0.authority_digest, second.0.authority_digest);
        assert_eq!(thread.authority_digest, thread_digest);
        assert_eq!(first.1["completeness"]["status"], "CONDITIONAL_TASK");
        assert_eq!(first.1["completeness"]["support"], "SUPPORTED");
        assert_eq!(first.1["completeness"]["coverage"], "PARTIAL");
        assert_eq!(first.1["completeness"]["certainty"], "UNSURE");
        assert_eq!(
            first.1["verificationObligations"].as_array().unwrap().len(),
            1
        );
    }

    #[test]
    fn retained_context_loads_after_thread_close_and_metadata_gc() {
        let (_temporary, state, thread, contexts) = fixture();
        let (authority, evidence, projection) = compose(
            &thread,
            "trace shared contract",
            &["shared".into()],
            ThreadContextBudgets::new(2).unwrap(),
            contexts.clone(),
        )
        .unwrap();
        let object = store_with_state(&state, &thread, authority, evidence, projection).unwrap();
        let mut replacement_contexts = contexts;
        replacement_contexts[0].context.context_id = format!("context:{}", digest('e'));
        let (replacement_authority, replacement_evidence, replacement_projection) = compose(
            &thread,
            "trace shared contract",
            &["shared".into()],
            ThreadContextBudgets::new(2).unwrap(),
            replacement_contexts,
        )
        .unwrap();
        let replacement = store_with_state(
            &state,
            &thread,
            replacement_authority,
            replacement_evidence,
            replacement_projection,
        )
        .unwrap();
        crate::thread::transition_with_state_for_test(
            &state,
            &thread,
            crate::thread::ThreadStatus::Closed,
        )
        .unwrap();
        crate::thread::transition_with_state_for_test(
            &state,
            &thread,
            crate::thread::ThreadStatus::GarbageCollected,
        )
        .unwrap();
        let loaded = load_with_state(&state, &thread, &object.context_id).unwrap();
        assert_eq!(loaded.evidence_digest, object.evidence_digest);
        assert!(
            canonical::bytes(&bounded_thread_context_stdout(&loaded).unwrap())
                .unwrap()
                .len()
                <= MAX_THREAD_STDOUT_BYTES
        );
        let original = object;
        let mut tampered = original.clone();
        tampered.projection = replacement.projection.clone();
        let root = state.thread_root(&thread.thread_id).unwrap();
        let record = root
            .join("contexts")
            .join(id_filename(&tampered.context_id).unwrap());
        state
            .write_private_atomic(&record, &canonical::bytes(&tampered).unwrap())
            .unwrap();
        assert!(load_with_state(&state, &thread, &tampered.context_id).is_err());

        let mut substituted = original.clone();
        substituted.evidence_ref = replacement.evidence_ref.clone();
        substituted.evidence_digest = replacement.evidence_digest.clone();
        state
            .write_private_atomic(&record, &canonical::bytes(&substituted).unwrap())
            .unwrap();
        assert!(load_with_state(&state, &thread, &substituted.context_id).is_err());

        state
            .write_private_atomic(&record, &canonical::bytes(&original).unwrap())
            .unwrap();
        assert!(load_with_state(&state, &thread, &original.context_id).is_ok());
    }

    #[test]
    fn concurrent_close_linearizes_after_admitted_context_publication() {
        let (_temporary, state, thread, contexts) = fixture();
        let (authority, evidence, projection) = compose(
            &thread,
            "trace shared contract",
            &["shared".into()],
            ThreadContextBudgets::new(2).unwrap(),
            contexts,
        )
        .unwrap();
        let admission = thread.admit_with_state(&state).unwrap();
        let close_state = state.clone();
        let close_thread = thread.clone();
        let (started_tx, started_rx) = std::sync::mpsc::channel();
        let (closed_tx, closed_rx) = std::sync::mpsc::channel();
        let closer = std::thread::spawn(move || {
            started_tx.send(()).unwrap();
            let result = crate::thread::transition_with_state_for_test(
                &close_state,
                &close_thread,
                crate::thread::ThreadStatus::Closed,
            );
            closed_tx.send(result).unwrap();
        });
        started_rx.recv().unwrap();
        assert!(
            closed_rx
                .recv_timeout(std::time::Duration::from_millis(50))
                .is_err()
        );
        let object = store_with_state(&state, &thread, authority, evidence, projection).unwrap();
        drop(admission);
        assert_eq!(
            closed_rx
                .recv_timeout(std::time::Duration::from_secs(2))
                .unwrap()
                .unwrap()
                .status,
            crate::thread::ThreadStatus::Closed
        );
        closer.join().unwrap();
        assert!(load_with_state(&state, &thread, &object.context_id).is_ok());
        assert!(thread.admit_with_state(&state).is_err());
    }

    #[test]
    fn fact_and_source_limits_are_global() {
        let (_temporary, _state, thread, mut contexts) = fixture();
        let template = contexts[0].context.evidence["context"]["matches"][0].clone();
        contexts[0].context.evidence["context"]["matches"] = Value::Array(
            (0..=MAX_THREAD_FACTS)
                .map(|index| {
                    let mut fact = template.clone();
                    fact["factKey"] = json!(format!("fact-{index:05}"));
                    fact
                })
                .collect(),
        );
        let composed = compose(
            &thread,
            "trace shared contract",
            &["shared".into()],
            ThreadContextBudgets::new(2).unwrap(),
            contexts,
        )
        .unwrap();
        assert!(composed.1["matches"].as_array().unwrap().len() <= MAX_THREAD_FACTS);
        assert_eq!(composed.1["selection"]["truncated"], true);
        assert!(canonical::bytes(&composed.1).unwrap().len() <= MAX_THREAD_EVIDENCE_BYTES);
        assert!(canonical::bytes(&composed.2).unwrap().len() <= THREAD_PROJECTION_TARGET_BYTES);

        let lanes = vec![
            (0..=MAX_THREAD_FACTS)
                .map(|index| json!({"index":index}))
                .collect::<Vec<_>>(),
        ];
        assert_eq!(fair_values(lanes, MAX_THREAD_FACTS).len(), MAX_THREAD_FACTS);
    }

    #[test]
    fn source_snippet_window_and_byte_limits_are_fail_closed() {
        let oversized = json!({
            "fileId":"src/main.py",
            "windows":[{"startLine":1,"endLine":1,"text":"x".repeat(MAX_THREAD_SNIPPET_BYTES + 1)}],
        });
        assert!(bounded_source(&oversized, MAX_THREAD_SNIPPET_BYTES).is_err());

        let split = json!({
            "fileId":"src/main.py",
            "contentRef":{"fixture":true},
            "startLine":1,
            "endLine":2,
            "text":"ignored",
            "windows":[
                {"startLine":1,"endLine":1,"text":"a".repeat(10 * 1024)},
                {"startLine":2,"endLine":2,"text":"b".repeat(10 * 1024)},
            ],
            "completeFile":true,
        });
        let (_, windows, bytes, truncated) =
            bounded_source(&split, MAX_THREAD_SNIPPET_BYTES).unwrap();
        assert_eq!(windows, 1);
        assert_eq!(bytes, 10 * 1024);
        assert!(truncated);

        let (_temporary, _state, _thread, fixture_contexts) = fixture();
        let base_member = fixture_contexts[0].member.clone();
        let mut many = fixture_contexts[0].clone();
        let mut facts = Vec::new();
        let mut sources = Vec::new();
        for index in 0..=MAX_THREAD_SOURCE_WINDOWS {
            let file = format!("src/file-{index:02}.py");
            facts.push(json!({
                "compilation":":/main",
                "factKey":format!("fact-{index:02}"),
                "domainUri":"language:test",
                "payload":{"file":file},
            }));
            sources.push(json!({
                "fileId":file,
                "contentRef":many.context.evidence_ref,
                "startLine":1,
                "endLine":1,
                "text":"x".repeat(10 * 1024),
                "windows":[{"startLine":1,"endLine":1,"text":"x".repeat(10 * 1024)}],
                "completeFile":true,
            }));
        }
        many.member = base_member;
        many.context.evidence["context"]["matches"] = Value::Array(facts);
        many.context.evidence["context"]["sources"] = Value::Array(sources);
        let (selected, total, limited) = select_sources(
            &[many],
            MAX_THREAD_SOURCE_WINDOWS,
            MAX_THREAD_SOURCE_BYTES,
            MAX_THREAD_SNIPPET_BYTES,
        )
        .unwrap();
        assert_eq!(total, MAX_THREAD_SOURCE_WINDOWS + 1);
        assert!(selected.len() < total);
        assert!(selected.len() <= MAX_THREAD_SOURCE_WINDOWS);
        assert!(limited);
        let selected_bytes = selected
            .iter()
            .flat_map(|source| source["windows"].as_array().unwrap())
            .map(|window| window["text"].as_str().unwrap().len())
            .sum::<usize>();
        assert!(selected_bytes <= MAX_THREAD_SOURCE_BYTES);
    }

    #[test]
    fn source_selection_honors_exact_window_and_byte_boundaries() {
        let (_temporary, _state, _thread, contexts) = fixture();
        let thirty_two = context_with_source_lengths(&contexts[0], &[1; MAX_THREAD_SOURCE_WINDOWS]);
        let (selected, total, limited) = select_sources(
            &[thirty_two],
            MAX_THREAD_SOURCE_WINDOWS,
            MAX_THREAD_SOURCE_BYTES,
            MAX_THREAD_SNIPPET_BYTES,
        )
        .unwrap();
        assert_eq!(total, MAX_THREAD_SOURCE_WINDOWS);
        assert_eq!(
            selected_source_usage(&selected),
            (MAX_THREAD_SOURCE_WINDOWS, 32)
        );
        assert!(!limited);

        let thirty_three =
            context_with_source_lengths(&contexts[0], &[1; MAX_THREAD_SOURCE_WINDOWS + 1]);
        let (selected, total, limited) = select_sources(
            &[thirty_three],
            MAX_THREAD_SOURCE_WINDOWS,
            MAX_THREAD_SOURCE_BYTES,
            MAX_THREAD_SNIPPET_BYTES,
        )
        .unwrap();
        assert_eq!(total, MAX_THREAD_SOURCE_WINDOWS + 1);
        assert_eq!(
            selected_source_usage(&selected),
            (MAX_THREAD_SOURCE_WINDOWS, 32)
        );
        assert!(limited);

        let sixteen = context_with_source_lengths(&contexts[0], &[16 * 1024; 16]);
        let (selected, total, limited) = select_sources(
            &[sixteen],
            MAX_THREAD_SOURCE_WINDOWS,
            MAX_THREAD_SOURCE_BYTES,
            MAX_THREAD_SNIPPET_BYTES,
        )
        .unwrap();
        assert_eq!(total, 16);
        assert_eq!(
            selected_source_usage(&selected),
            (16, MAX_THREAD_SOURCE_BYTES)
        );
        assert!(!limited);

        let mut byte_lengths = vec![16 * 1024; 16];
        byte_lengths.push(1);
        let plus_one = context_with_source_lengths(&contexts[0], &byte_lengths);
        let (selected, total, limited) = select_sources(
            &[plus_one],
            MAX_THREAD_SOURCE_WINDOWS,
            MAX_THREAD_SOURCE_BYTES,
            MAX_THREAD_SNIPPET_BYTES,
        )
        .unwrap();
        assert_eq!(total, 17);
        assert_eq!(
            selected_source_usage(&selected),
            (16, MAX_THREAD_SOURCE_BYTES)
        );
        assert!(limited);
    }

    #[test]
    fn oversized_member_fact_does_not_starve_later_bounded_evidence() {
        let (_temporary, _state, thread, mut contexts) = fixture();
        contexts.sort_by(|left, right| left.member.member_alias.cmp(&right.member.member_alias));
        let small = contexts[0].context.evidence["context"]["matches"][0].clone();
        let mut later = small.clone();
        later["factKey"] = json!("later-bounded");
        let mut oversized = small;
        oversized["factKey"] = json!("oversized");
        oversized["payload"]["opaque"] = json!("x".repeat(MAX_THREAD_EVIDENCE_BYTES));
        contexts[0].context.evidence["context"]["matches"] = json!([oversized, later]);
        let composed = compose(
            &thread,
            "trace shared contract",
            &["shared".into()],
            ThreadContextBudgets::new(2).unwrap(),
            contexts,
        )
        .unwrap();
        let keys = composed.1["matches"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|fact| fact["factKey"].as_str())
            .collect::<BTreeSet<_>>();
        assert!(keys.contains("later-bounded"));
        assert!(!keys.contains("oversized"));
        assert_eq!(composed.1["selection"]["truncated"], true);
    }

    #[test]
    fn oversized_lane_does_not_starve_other_member_or_compilation_lanes() {
        let temporary = tempfile::tempdir().unwrap();
        let state = StateAuthority::open(temporary.path().join("v2")).unwrap();
        let mut left_session = session('a', "repo:left", SessionLanguage::Python);
        left_session.compilations = vec![":/main".into(), ":/test".into()];
        let mut unsigned = left_session.clone();
        unsigned.authority_digest.clear();
        left_session.authority_digest = canonical::hash(&unsigned).unwrap();
        let mut right_session = session('b', "repo:right", SessionLanguage::Python);
        right_session.compilations = vec![":/main".into(), ":/test".into()];
        let mut unsigned = right_session.clone();
        unsigned.authority_digest.clear();
        right_session.authority_digest = canonical::hash(&unsigned).unwrap();
        let left = member("left", left_session);
        let right = member("right", right_session);
        let thread = create_with_state(
            &state,
            "thread:fair-lanes".into(),
            1,
            vec![left.clone(), right.clone()],
        )
        .unwrap();
        let mut left_context = context(&left, 'c', "COMPLETE_TASK");
        let mut right_context = context(&right, 'd', "COMPLETE_TASK");
        let template = left_context.context.evidence["context"]["matches"][0].clone();
        let fact = |key: &str, compilation: &str| {
            let mut value = template.clone();
            value["factKey"] = json!(key);
            value["compilation"] = json!(compilation);
            value
        };
        let mut oversized = fact("left-main-oversized", ":/main");
        oversized["payload"]["opaque"] = json!("x".repeat(MAX_THREAD_EVIDENCE_BYTES));
        left_context.context.evidence["context"]["matches"] =
            json!([oversized, fact("left-test-small", ":/test")]);
        right_context.context.evidence["context"]["matches"] = json!([
            fact("right-main-small", ":/main"),
            fact("right-test-small", ":/test")
        ]);
        let composed = compose(
            &thread,
            "trace shared contract",
            &["shared".into()],
            ThreadContextBudgets::new(2).unwrap(),
            vec![left_context, right_context],
        )
        .unwrap();
        for rows in [&composed.1["matches"], &composed.2["matches"]] {
            let keys = rows
                .as_array()
                .unwrap()
                .iter()
                .filter_map(|fact| fact["factKey"].as_str())
                .collect::<BTreeSet<_>>();
            assert_eq!(
                keys,
                BTreeSet::from(["left-test-small", "right-main-small", "right-test-small"])
            );
        }
    }

    #[test]
    fn frozen_byte_and_source_limits_accept_limit_and_reject_limit_plus_one() {
        let evidence_at =
            canonical::bytes(&json!("x".repeat(MAX_THREAD_EVIDENCE_BYTES - 2))).unwrap();
        let evidence_over =
            canonical::bytes(&json!("x".repeat(MAX_THREAD_EVIDENCE_BYTES - 1))).unwrap();
        assert_eq!(evidence_at.len(), MAX_THREAD_EVIDENCE_BYTES);
        assert_eq!(evidence_over.len(), MAX_THREAD_EVIDENCE_BYTES + 1);
        validate_evidence_bytes(&evidence_at).unwrap();
        assert!(validate_evidence_bytes(&evidence_over).is_err());

        // println! adds one LF after the rendered JSON.
        let stdout_at = canonical::bytes(&json!("x".repeat(MAX_THREAD_STDOUT_BYTES - 3))).unwrap();
        let stdout_over =
            canonical::bytes(&json!("x".repeat(MAX_THREAD_STDOUT_BYTES - 2))).unwrap();
        assert_eq!(stdout_at.len() + 1, MAX_THREAD_STDOUT_BYTES);
        assert_eq!(stdout_over.len() + 1, MAX_THREAD_STDOUT_BYTES + 1);
        validate_stdout_bytes(&stdout_at).unwrap();
        assert!(validate_stdout_bytes(&stdout_over).is_err());

        let budgets = ThreadContextBudgets::new(1).unwrap();
        validate_source_budget(&budgets, MAX_THREAD_SOURCE_WINDOWS, 0).unwrap();
        assert!(validate_source_budget(&budgets, MAX_THREAD_SOURCE_WINDOWS + 1, 0).is_err());
        validate_source_budget(&budgets, 0, MAX_THREAD_SOURCE_BYTES).unwrap();
        assert!(validate_source_budget(&budgets, 0, MAX_THREAD_SOURCE_BYTES + 1).is_err());
    }
}
