//! Retained single-repository comparison, independent of thread pair authority.
use crate::canonical;
use crate::cas::{CasObject, CasStore};
use crate::error::{ClewError, ErrorCode};
use crate::generation_service::{self, AnalysisExecutionAuthority, ReadyGenerationSet};
use crate::generation_v2::GenerationManifest;
use crate::repository_snapshot::{
    self, RepositoryInputSnapshot, TrackedScopeLimits, WorkingTreeLimits,
};
use crate::session::{ModelCachePolicy, SessionAuthority, SessionLanguage};
use crate::state::StateAuthority;
use crate::working_tree_change::{self as change, Analysis, Comparison, Declaration, Side};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

const ROOT_SCHEMA: &str = "codeclew-working-tree-change-root/1.0";
const MAX_FACTS: u64 = 131_072;
const MAX_PAYLOAD_BYTES: usize = 64 * 1024 * 1024;

pub struct InspectRequest {
    pub repository: PathBuf,
    pub target_ref: String,
    pub language: SessionLanguage,
    pub compilations: Vec<String>,
    pub profile_id: String,
    pub generation_jobs: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChangeRoot {
    pub schema: String,
    pub comparison_id: String,
    pub report: CasObject,
}

pub fn inspect(request: InspectRequest) -> Result<Value, ClewError> {
    let state = StateAuthority::process_default()?;
    let store = CasStore::open(&state)?;
    let after = SessionAuthority::open_with_source(
        &request.repository,
        &request.target_ref,
        request.language,
        &request.compilations,
        request.generation_jobs,
        ModelCachePolicy::NonCacheable,
        None,
        Some(&request.profile_id),
    )?;
    let mut before = None;
    let result = (|| {
        let limits = WorkingTreeLimits::default();
        // Bound HEAD blobs before creating a detached checkout, including when
        // a large or unsupported HEAD input was removed from the current index.
        let (base_inputs, _) = repository_snapshot::capture_commit_scope(
            &request.repository,
            &after.base_revision,
            &[".".into()],
            &store,
            |path| {
                !path
                    .split('/')
                    .any(|component| component == ".semantic-thread")
            },
            TrackedScopeLimits {
                max_files: limits.max_files,
                max_file_bytes: limits.max_file_bytes as usize,
                max_total_bytes: limits.max_total_bytes as usize,
                max_tree_entries: limits.max_files,
                max_tree_bytes: limits.max_inventory_bytes,
                max_tree_path_bytes: limits.max_path_bytes,
            },
        )?;
        if base_inputs
            .index
            .iter()
            .any(|entry| !matches!(entry.mode, 0o100644 | 0o100755))
        {
            return Err(invalid(
                "comparison base contains unsupported links or Git inputs",
            ));
        }
        before = Some(SessionAuthority::open(
            &request.repository,
            &request.target_ref,
            request.language,
            &request.compilations,
            request.generation_jobs,
            ModelCachePolicy::NonCacheable,
            None,
        )?);
        let before_session = before.as_ref().unwrap();
        if before_session.base_revision != after.base_revision {
            return Err(ClewError::new(
                ErrorCode::InputMutated,
                "HEAD changed between before/after capture; retry inspect",
            ));
        }
        let (_, before_snapshot, before_object) =
            repository_snapshot::capture_working_tree(&before_session.repository_path()?, &store)?;
        let after_snapshot = after.working_tree_snapshot(&store)?;
        let after_object = after.working_tree.as_ref().unwrap().snapshot.clone();
        let before_analysis = analyze(&store, before_session, &before_snapshot);
        let after_analysis = analyze(&store, &after, &after_snapshot);
        let report = change::compare(
            &store,
            Side {
                profile_id: request.profile_id.clone(),
                session: before_session.clone(),
                snapshot: before_object,
                analysis: before_analysis,
            },
            Side {
                profile_id: request.profile_id.clone(),
                session: after.clone(),
                snapshot: after_object,
                analysis: after_analysis,
            },
            &before_snapshot,
            &after_snapshot,
        )?;
        let bytes = canonical::bytes(&report).map_err(internal)?;
        if bytes.len() > change::MAX_REPORT_BYTES {
            return Err(resource("comparison evidence exceeds 64 MiB"));
        }
        let object = store.put(change::SCHEMA, &bytes)?;
        let root = ChangeRoot {
            schema: ROOT_SCHEMA.into(),
            comparison_id: report.comparison_id.clone(),
            report: object,
        };
        let path = root_path(&state, &root.comparison_id)?;
        state.write_private_atomic(&path, &canonical::bytes(&root).map_err(internal)?)?;
        bounded_stdout(&root, &report)
    })();
    // Comparison roots retain the source/analysis closure before releasing the
    // derived session worktrees. A failed compiler still yields textual evidence.
    let mut cleanup = Vec::new();
    for session in before.iter().chain(std::iter::once(&after)) {
        let outcome = session.close().and_then(|_| session.gc(false));
        cleanup.push(json!({"sessionId":session.session_id,
            "status":if outcome.is_ok() { "COLLECTED" } else { "DEFERRED" },
            "failure":outcome.err().map(|error| json!({"code":error.code,"message":error.message}))}));
    }
    result.map(|mut result| {
        result["cleanup"] = json!(cleanup);
        result
    })
}

fn analyze(
    store: &CasStore,
    session: &SessionAuthority,
    snapshot: &RepositoryInputSnapshot,
) -> Analysis {
    let authority = if session.language == SessionLanguage::Kotlin {
        "KOTLIN_COMPILER_SUPPORTED_SUBSET"
    } else {
        "RUST_SYNTAX_ONLY"
    };
    let ready = match generation_service::ensure_session_generation(session) {
        Ok(ready) => ready,
        Err(error) => return failed_analysis(authority, None, error),
    };
    match collect_declarations(store, session, snapshot, &ready) {
        Ok((declarations, boundaries)) => Analysis {
            status: "AVAILABLE".into(),
            authority: authority.into(),
            declaration_coverage_complete: boundaries.is_empty()
                && !declarations.is_empty()
                && (session.language != SessionLanguage::Kotlin
                    || declarations.iter().all(|d| d.complete_shape)),
            ready: Some(ready),
            declarations,
            boundaries,
            failure: None,
        },
        Err(error) => failed_analysis(authority, Some(ready), error),
    }
}

fn failed_analysis(
    authority: &str,
    ready: Option<ReadyGenerationSet>,
    error: ClewError,
) -> Analysis {
    Analysis {
        status: "FAILED".into(),
        authority: authority.into(),
        ready,
        declarations: vec![],
        declaration_coverage_complete: false,
        boundaries: vec![],
        failure: Some(json!({"code":error.code,"message":error.message})),
    }
}

fn collect_declarations(
    store: &CasStore,
    session: &SessionAuthority,
    snapshot: &RepositoryInputSnapshot,
    ready: &ReadyGenerationSet,
) -> Result<(Vec<Declaration>, Vec<Value>), ClewError> {
    let sources = change::source_files(snapshot);
    let mut cache = change::SourceCache::default();
    let mut declarations = Vec::new();
    let mut boundaries = Vec::new();
    let mut count = 0u64;
    let mut bytes = 0usize;
    for compilation in &ready.compilations {
        if session.language == SessionLanguage::Kotlin
            && compilation.incremental.analysis_execution_authority
                != AnalysisExecutionAuthority::CompilerWorker
        {
            return Err(invalid(
                "Kotlin comparison requires compiler-worker evidence",
            ));
        }
        let generation: GenerationManifest =
            read_canonical(store, &compilation.generation, 16 * 1024 * 1024)?;
        if generation.derived_input_manifest != compilation.derived_input_manifest
            || compilation.repository_snapshot != ready.repository_snapshot
        {
            return Err(invalid(
                "generation input binding disagrees with ready authority",
            ));
        }
        count = count
            .checked_add(generation.fact_count)
            .ok_or_else(|| resource("fact count overflow"))?;
        if count > MAX_FACTS {
            return Err(resource("comparison exceeds generation fact visit budget"));
        }
        generation.visit_facts(store, |fact| {
            bytes = bytes.checked_add(fact.payload.size as usize).ok_or_else(|| resource("payload count overflow"))?;
            if bytes > MAX_PAYLOAD_BYTES || fact.payload.size > 1024 * 1024 { return Err(resource("comparison exceeds semantic payload budget")); }
            let payload: Value = read_canonical(store, &fact.payload, 1024 * 1024)?;
            let kotlin = session.language == SessionLanguage::Kotlin;
            let schema = payload.get("schema").and_then(Value::as_str).unwrap_or("");
            if kotlin && matches!(schema, "declaration-descriptor-boundary/0.1" | "declaration-relation-boundary/0.1") {
                crate::semantic_validation::validate_kotlin_semantic_payload(&payload)?;
                if boundaries.len() < change::MAX_DECLARATIONS { boundaries.push(payload); }
                return Ok(());
            }
            let declaration = if kotlin { schema == "declaration-descriptor/0.1" } else { payload["kind"] == "declaration" };
            if !declaration { return Ok(()); }
            if kotlin { crate::semantic_validation::validate_kotlin_semantic_payload(&payload)?; }
            if declarations.len() == change::MAX_DECLARATIONS { return Err(resource("comparison exceeds declaration budget")); }
            let file = string(&payload, "file")?;
            let source = sources.get(file).ok_or_else(|| invalid("declaration source is absent from its input snapshot"))?;
            let start = offset(&payload, if kotlin { "start" } else { "rangeStart" })?;
            let end = offset(&payload, if kotlin { "end" } else { "rangeEnd" })?;
            if start >= end { return Err(invalid("declaration source range is invalid")); }
            let anchor = cache.anchor(store, file, &source.content, start, end)?;
            let symbol = string(&payload, "symbolIdentity")?.to_owned();
            let kind = string(&payload, "declarationKind")?.to_owned();
            let family = if kotlin { payload.get("compilerCallableId").and_then(Value::as_str).map(str::to_owned) }
                else { Some(format!("syntax:{file}#{kind}:{}", string(&payload, "name")?)) };
            let projected_shape = if kotlin { crate::thread_callables::projected_payload(&payload) }
                else { json!({"name":payload["name"], "declarationKind":kind, "cfgStatus":payload["cfgStatus"]}) };
            declarations.push(Declaration { compilation: compilation.compilation.clone(), symbol, family, kind,
                source: anchor,
                fact_key: fact.fact_key.clone(), payload: fact.payload.clone(), projected_shape,
                authority: if kotlin { "COMPILER_PROJECTED_DECLARATION" } else { "SYNTAX_DECLARATION" }.into(),
                complete_shape: kotlin && payload.get("attributeCoverage").is_none() && payload.get("sourceRowHash").is_none(),
            });
            Ok(())
        })?;
    }
    declarations.sort_by(|a, b| (&a.compilation, &a.symbol).cmp(&(&b.compilation, &b.symbol)));
    if declarations
        .windows(2)
        .any(|pair| pair[0].compilation == pair[1].compilation && pair[0].symbol == pair[1].symbol)
    {
        return Err(invalid(
            "declaration identity is duplicated in a selected compilation",
        ));
    }
    if !boundaries.is_empty() {
        for declaration in &mut declarations {
            declaration.complete_shape = false;
        }
    }
    Ok((declarations, boundaries))
}

pub fn load(comparison_id: &str) -> Result<(ChangeRoot, Comparison), ClewError> {
    let state = StateAuthority::process_default()?;
    let store = CasStore::open(&state)?;
    let bytes = state.read_private_file(&root_path(&state, comparison_id)?, 64 * 1024)?;
    let root: ChangeRoot = serde_json::from_slice(&bytes).map_err(internal)?;
    if root.schema != ROOT_SCHEMA
        || root.comparison_id != comparison_id
        || root.report.object_schema != change::SCHEMA
    {
        return Err(invalid("retained comparison root is invalid"));
    }
    let report: Comparison = read_canonical(&store, &root.report, change::MAX_REPORT_BYTES)?;
    report.verify()?;
    if report.comparison_id != comparison_id {
        return Err(invalid("comparison root refers to another report"));
    }
    Ok((root, report))
}

pub fn forget(comparison_id: &str) -> Result<Value, ClewError> {
    let (root, _) = load(comparison_id)?;
    let state = StateAuthority::process_default()?;
    let path = root_path(&state, comparison_id)?;
    state
        .directory(Path::new("changes"))?
        .remove_file(path.file_name().unwrap())?;
    Ok(
        json!({"schema":"codeclew-change-forget/1.0", "comparisonId":root.comparison_id, "status":"FORGOTTEN", "sourceFilesChanged":false}),
    )
}

pub fn show(comparison_id: &str) -> Result<Value, ClewError> {
    let (root, report) = load(comparison_id)?;
    bounded_stdout(&root, &report)
}

fn model_bindings(analysis: &Analysis) -> Vec<Value> {
    analysis.ready.iter().flat_map(|ready| &ready.compilations).map(|compilation|
        json!({"compilation":compilation.compilation,"derivedInputManifest":compilation.derived_input_manifest,
            "generation":compilation.generation,"coverage":compilation.coverage,"certainty":compilation.certainty,
            "obligations":compilation.obligations})).collect()
}

pub fn bounded_stdout(root: &ChangeRoot, report: &Comparison) -> Result<Value, ClewError> {
    let mut result = json!({"schema":"codeclew-change-inspect/1.0", "comparisonId":root.comparison_id,
        "report":root.report, "status":report.status,
        "sourceSelection":"WORKING_TREE", "baseRevision":report.before.session.base_revision,
        "beforeSnapshot":report.before.snapshot,"afterSnapshot":report.after.snapshot,
        "compilations":report.after.session.compilations,"profileId":report.after.profile_id,
        "modelAuthority":{"before":model_bindings(&report.before.analysis),"after":model_bindings(&report.after.analysis),"equivalence":"NOT_ASSUMED_ACROSS_DIFFERENT_INPUTS"},
        "coverage":{"beforeDeclarationsComplete":report.before.analysis.declaration_coverage_complete,"afterDeclarationsComplete":report.after.analysis.declaration_coverage_complete,"beforeBoundaries":report.before.analysis.boundaries.len(),"afterBoundaries":report.after.analysis.boundaries.len()},
        "beforeAnalysis":{"status":report.before.analysis.status,"authority":report.before.analysis.authority,"failure":report.before.analysis.failure},
        "afterAnalysis":{"status":report.after.analysis.status,"authority":report.after.analysis.authority,"failure":report.after.analysis.failure},
        "comparability":report.comparability,"obligations":report.obligations,"testsExecuted":false,
        "counts":{"changedFiles":report.total_changed_file_count,"changedDeclarations":report.total_changed_declaration_count,
            "unchangedDeclarations":report.unchanged_declaration_count,"omittedFiles":report.omitted_file_count,"omittedDeclarations":report.omitted_declaration_count},
        "files":report.files.iter().take(32).collect::<Vec<_>>(),
        "declarations":report.declarations.iter().take(24).collect::<Vec<_>>()});
    loop {
        let file_count = result["files"].as_array().unwrap().len();
        let declaration_count = result["declarations"].as_array().unwrap().len();
        result["stdoutOmittedFiles"] =
            json!(report.total_changed_file_count.saturating_sub(file_count));
        result["stdoutOmittedDeclarations"] = json!(
            report
                .total_changed_declaration_count
                .saturating_sub(declaration_count)
        );
        if canonical::bytes(&result).map_err(internal)?.len() <= 56 * 1024 {
            return Ok(result);
        }
        if declaration_count > 0 {
            result["declarations"].as_array_mut().unwrap().pop();
        } else if file_count > 0 {
            result["files"].as_array_mut().unwrap().pop();
        } else {
            return Err(resource("comparison summary exceeds stdout budget"));
        }
    }
}

fn root_path(state: &StateAuthority, id: &str) -> Result<PathBuf, ClewError> {
    let component = id
        .strip_prefix("comparison:sha256:")
        .ok_or_else(|| invalid("comparison id prefix is invalid"))?;
    if component.len() != 64
        || !component
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
    {
        return Err(invalid("comparison id digest is invalid"));
    }
    Ok(state
        .directory(Path::new("changes"))?
        .path()
        .join(format!("{component}.json")))
}

fn read_canonical<T: serde::de::DeserializeOwned + Serialize>(
    store: &CasStore,
    object: &CasObject,
    limit: usize,
) -> Result<T, ClewError> {
    let lease = store.read(object, limit)?;
    let value: T = serde_json::from_slice(lease.bytes()).map_err(internal)?;
    if canonical::bytes(&value).map_err(internal)? != lease.bytes() {
        return Err(invalid("retained comparison evidence is not canonical"));
    }
    Ok(value)
}
fn string<'a>(value: &'a Value, key: &str) -> Result<&'a str, ClewError> {
    value
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| invalid("declaration string field is missing"))
}
fn offset(value: &Value, key: &str) -> Result<usize, ClewError> {
    value
        .get(key)
        .and_then(Value::as_u64)
        .and_then(|v| usize::try_from(v).ok())
        .ok_or_else(|| invalid("declaration offset is missing"))
}
fn invalid(message: &str) -> ClewError {
    ClewError::new(ErrorCode::InvalidInput, message)
}
fn resource(message: &str) -> ClewError {
    ClewError::new(ErrorCode::ResourceLimit, message)
}
fn internal(message: impl std::fmt::Display) -> ClewError {
    ClewError::new(ErrorCode::Internal, message.to_string())
}
