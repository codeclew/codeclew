//! Current JVM evidence uses the normal admitted session and immutable generation.
use super::{
    bytes, digest, invalid, io_error,
    model::*,
    store::{self, Repository},
};
use crate::{
    canonical,
    cas::CasStore,
    error::{ClewError, ErrorCode},
    generation_service,
    generation_v2::GenerationManifest,
    operations::{self, DoctorOperation, DoctorScope, DoctorTask},
    repository_snapshot::isolated_git_command,
    runtime::RuntimeAuthority,
    session::{ModelCachePolicy, SessionAuthority, SessionLanguage},
    spring_entrypoints,
    state::StateAuthority,
};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    ops::Range,
    path::{Path, PathBuf},
    process::Stdio,
    sync::Arc,
};

const MAX_EVIDENCE: u64 = 256 * 1024 * 1024;
const MAX_FACTS: usize = 524_288;

pub fn git(repo: &Path, args: &[&str]) -> Result<String, ClewError> {
    let result = isolated_git_command(repo)
        .args(args)
        .stdin(Stdio::null())
        .output()
        .map_err(io_error)?;
    if !result.status.success() || result.stdout.len() > 1024 * 1024 {
        return Err(invalid(
            "bound repository identity or selected revision is unavailable",
        ));
    }
    String::from_utf8(result.stdout)
        .map(|s| s.trim_end_matches('\n').to_owned())
        .map_err(io_error)
}

fn locator(value: &str) -> Option<String> {
    if store::safe_url(value) {
        return Some(value.trim_end_matches('/').trim_end_matches(".git").into());
    }
    if let Some(rest) = value.strip_prefix("ssh://git@") {
        let (authority, path) = rest.split_once('/')?;
        let host = if let Some((host, port)) = authority.split_once(':') {
            let _ = port.parse::<u16>().ok().filter(|port| *port > 0)?;
            host
        } else {
            authority
        };
        // The registered identity is a web locator; an SSH transport port is not a web port.
        let candidate = format!("https://{host}/{path}");
        if store::safe_url(&candidate) {
            return Some(candidate.trim_end_matches(".git").into());
        }
        return None;
    }
    if let Some(rest) = value.strip_prefix("git@") {
        let (host, path) = rest.split_once(':')?;
        let candidate = format!("https://{host}/{path}");
        if store::safe_url(&candidate) {
            return Some(candidate.trim_end_matches(".git").into());
        }
    }
    None
}

pub fn verify_repository(service: &Service, repo: &Path) -> Result<(), ClewError> {
    let remote = git(repo, &["remote", "get-url", "origin"])?;
    if locator(&remote) != locator(&service.repository) {
        return Err(ClewError::new(
            ErrorCode::BindingChanged,
            "local checkout origin does not match the registered credential-free repository identity",
        ));
    }
    Ok(())
}

pub fn bind(repository: &Repository, id: &str, source: &Path) -> Result<Value, ClewError> {
    let services = repository.services()?;
    let service = services.get(id).ok_or_else(|| invalid("unknown service"))?;
    let path = source.canonicalize().map_err(io_error)?;
    if path.starts_with(&repository.root) || repository.root.starts_with(&path) {
        return Err(invalid(
            "documentation and service source checkouts must have separate roots",
        ));
    }
    verify_repository(service, &path)?;
    let binding = LocalBinding {
        schema: "codeclew-documentation-local-binding/1.0".into(),
        service: id.into(),
        repository: path
            .to_str()
            .ok_or_else(|| invalid("checkout path is not UTF-8"))?
            .into(),
    };
    let _lock = repository.lock()?;
    repository.atomic(&format!(".codeclew/bindings/{id}.json"), &bytes(&binding)?)?;
    Ok(
        json!({"schema":"codeclew-docs-bind/1.0","status":"BOUND","service":id,"repositoryId":service.repository_id}),
    )
}

pub fn bound_repository(repository: &Repository, service: &Service) -> Result<PathBuf, ClewError> {
    let binding: LocalBinding = store::read(
        &repository.path(&format!(".codeclew/bindings/{}.json", service.id))?,
        store::MAX_RECORD,
    )?;
    if binding.schema != "codeclew-documentation-local-binding/1.0" || binding.service != service.id
    {
        return Err(invalid("local binding identity is invalid"));
    }
    let path = PathBuf::from(binding.repository)
        .canonicalize()
        .map_err(io_error)?;
    verify_repository(service, &path)?;
    Ok(path)
}

pub fn capture(repository: &Repository, service: &Service) -> Result<ServiceEvidence, ClewError> {
    capture_with_diagnostics(repository, service, None)
}

pub(crate) fn capture_with_diagnostics(
    repository: &Repository,
    service: &Service,
    debug_output: Option<&crate::maven_diagnostics::DebugOutput>,
) -> Result<ServiceEvidence, ClewError> {
    if let Some(evidence) = super::evidence_package::selected(repository, service)? {
        return Ok(evidence);
    }
    capture_local_with_diagnostics(repository, service, debug_output)
}

/// Source selection consumes the supplied admission policy. It must not follow
/// a subsequently edited policy while reporting the original captured inputs.
pub(super) fn capture_with_expectation(
    repository: &Repository,
    service: &Service,
    expectation: Option<&super::evidence_package::Expectation>,
    debug_output: Option<&crate::maven_diagnostics::DebugOutput>,
) -> Result<ServiceEvidence, ClewError> {
    if let Some(evidence) =
        super::evidence_package::selected_with_expectation(repository, service, expectation)?
    {
        return Ok(evidence);
    }
    capture_local_with_diagnostics(repository, service, debug_output)
}

pub(super) fn capture_local(
    repository: &Repository,
    service: &Service,
) -> Result<ServiceEvidence, ClewError> {
    capture_local_with_diagnostics(repository, service, None)
}

fn capture_local_with_diagnostics(
    repository: &Repository,
    service: &Service,
    debug_output: Option<&crate::maven_diagnostics::DebugOutput>,
) -> Result<ServiceEvidence, ClewError> {
    let repo = bound_repository(repository, service)?;
    if service.profile == "source-syntax" {
        // Pure syntax can reuse this deterministic key. An enabled semantic
        // provider adds external authority that this key does not contain;
        // even an older manifest marked REUSABLE must not bypass admission.
        let revision = git(
            &repo,
            &[
                "rev-parse",
                "--verify",
                &format!("{}^{{commit}}", service.target_ref),
            ],
        )?;
        let service_digest = digest(service)?;
        let cache_key = digest(&json!([
            &revision,
            &service_digest,
            SOURCE_EXTRACTOR,
            service.language
        ]))?;
        let semantic = super::modules::semantic(service);
        let semantic_enabled = semantic.is_some();
        if !semantic_enabled
            && let Some(reused) =
                super::cache::load_capture_if_valid(repository, &service.id, &cache_key)?
        {
            return Ok(reused);
        }
        let mut source = super::progress::run("ACQUIRE_SYNTAX_EVIDENCE", || {
            super::syntax::capture(service, &repo)
        })?;
        if let Some(semantic) = semantic {
            let mut provider = service.clone();
            provider.source = None;
            provider.modules = None;
            provider.profile = semantic.profile.clone();
            provider.compilations = vec![semantic.compilation.clone()];
            super::syntax::enrich(
                &mut source,
                capture_local_with_diagnostics(repository, &provider, debug_output),
            )?;
        }
        super::contracts::capture(service, &repo, &mut source)?;
        super::modules::attach(service, &mut source)?;
        let _lock = repository.lock()?;
        if semantic_enabled {
            let mut manifest = super::cache::store_capture(repository, &source)?;
            super::cache::mark_non_cacheable(
                &mut manifest,
                "semantic provider capture lacks complete external authority and must not be reused",
            );
            let cache_path = format!(
                ".codeclew/cache/{}-{}.json",
                service.id,
                cache_key.trim_start_matches("sha256:")
            );
            repository.atomic(&cache_path, &bytes(&manifest)?)?;
        } else {
            super::cache::save_capture(repository, &service.id, &cache_key, &source)?;
        }
        return Ok(source);
    }
    let runtime = RuntimeAuthority::from_environment()?
        .ok_or_else(|| invalid("documentation analysis requires the supported clew launcher"))?;
    let compilations = service.effective_compilations();
    if compilations.is_empty() {
        return Err(invalid(
            "service requires at least one compilation selector",
        ));
    }
    let language = if service.language == "kotlin" {
        SessionLanguage::Kotlin
    } else {
        SessionLanguage::Java
    };
    let readiness = super::progress::run("SOURCE_ADMISSION", || {
        operations::doctor(
            &runtime,
            DoctorScope::Task,
            Some(&repo),
            Some(&service.target_ref),
            Some(DoctorTask {
                language,
                profile_id: &service.profile,
                operation: DoctorOperation::Analysis,
                compilations: &compilations,
                committed: false,
                working_tree: false,
                maven_settings: None,
            }),
        )
    })?;
    if readiness["status"] != "PASS" {
        let action = readiness["nextAction"]
            .as_str()
            .unwrap_or("RUN_TASK_DOCTOR");
        return Err(ClewError::new(
            ErrorCode::PreconditionFailed,
            format!("documentation source admission requires action: {action}"),
        ));
    }
    let revision = git(&repo, &["rev-parse", "--verify", "HEAD^{commit}"])?;
    let service_digest = digest(service)?;
    // Revalidate current admission and repository state even when cached bytes exist.
    let cache_key = digest(&json!([
        &revision,
        &service_digest,
        EXTRACTOR,
        service.language,
        runtime.runtime_key
    ]))?;
    let cache_path = format!(
        ".codeclew/cache/{}-{}.json",
        service.id,
        cache_key.trim_start_matches("sha256:")
    );
    let session = super::progress::run("OPEN_ANALYSIS_SESSION", || {
        SessionAuthority::open(
            &repo,
            &service.target_ref,
            language,
            &compilations,
            None,
            ModelCachePolicy::NonCacheable,
            None,
        )
    })?;
    let result = super::progress::run("ACQUIRE_COMPILER_EVIDENCE", || {
        capture_session(&session, service, &service_digest, debug_output)
    });
    // Only use supported lifecycle operations; documentation records have no session dependency.
    let cleanup = super::progress::run("CLOSE_ANALYSIS_SESSION", || {
        session.abort().and_then(|_| session.gc(false)).map(|_| ())
    });
    let mut evidence = match result {
        Ok(evidence) => evidence,
        Err(error) => {
            cleanup?;
            return Err(error);
        }
    };
    super::contracts::capture(service, &repo, &mut evidence)?;
    super::modules::attach(service, &mut evidence)?;
    cleanup?;
    if git(&repo, &["rev-parse", "--verify", "HEAD^{commit}"])? != revision {
        return Err(ClewError::new(
            ErrorCode::InputMutated,
            "service revision changed during documentation extraction",
        ));
    }
    let _lock = super::progress::run("WAIT_CAPTURE_STORE_LOCK", || repository.lock())?;
    // New-format captures persist a small reference envelope: the heavy
    // payload lives once in the immutable object store, and the keyed cache
    // path holds validated object references instead of a duplicated full
    // ServiceEvidence serialization.
    let mut manifest = super::progress::run("STORE_CAPTURE", || {
        super::cache::store_capture(repository, &evidence)
    })?;
    // Maven/external-state capture has no complete build/settings/dependency
    // authority in the reuse key, so it is explicitly non-cacheable (recapture
    // on the next run) rather than silently reusable.
    super::cache::mark_non_cacheable(
        &mut manifest,
        "Maven/external-state capture lacks complete build/settings/dependency authority",
    );
    repository.atomic(&cache_path, &bytes(&manifest)?)?;
    Ok(evidence)
}

fn capture_session(
    session: &SessionAuthority,
    service: &Service,
    service_digest: &str,
    debug_output: Option<&crate::maven_diagnostics::DebugOutput>,
) -> Result<ServiceEvidence, ClewError> {
    let ready = super::progress::run("ENSURE_COMPILER_GENERATION", || {
        generation_service::ensure_session_generation_with_diagnostics(
            session,
            debug_output,
            generation_service::wants_writable_then_seal(&service.profile),
            &service.annotation_processor_paths,
        )
    })?;
    let state = StateAuthority::process_default()?;
    let store = CasStore::open(&state)?;
    // A writable-then-seal generation persists the exact transformed source
    // bytes it was indexed against. Documentation must read those bytes, never
    // slice the original repository snapshot with transformed coordinates.
    // Each admitted compilation carries its own transformed reference, so the
    // source table is keyed per compilation scope instead of sharing the
    // set-level first-present reference across all scopes.
    let snapshot = generation_service::load_snapshot(&store, &ready)?;
    let mut snapshot_text: BTreeMap<String, Arc<str>> = BTreeMap::new();
    for entry in &snapshot.worktree {
        if let Some(reference) = &entry.content {
            let lease = store.read(reference, store::MAX_RECORD as usize)?;
            if let Ok(text) = String::from_utf8(lease.bytes().to_vec()) {
                snapshot_text.insert(entry.path.clone(), Arc::from(text));
            }
        }
    }
    let mut facts = Vec::new();
    let mut count = 0usize;
    let mut total = 0u64;
    for compilation in &ready.compilations {
        let lease = store.read(&compilation.generation, MAX_EVIDENCE as usize)?;
        let generation: GenerationManifest =
            serde_json::from_slice(lease.bytes()).map_err(io_error)?;
        let scope = json!({"compilation": compilation.compilation});
        generation.visit_facts(&store, |fact| {
            let domain = if service.language == "kotlin" {
                "analysis:kotlin-semantic-facts"
            } else {
                "analysis:java-compiler-facts"
            };
            if fact.domain_uri.as_str() != domain {
                return Ok(());
            }
            count += 1;
            total = total.saturating_add(fact.payload.size);
            if count > MAX_FACTS || total > MAX_EVIDENCE {
                return Err(ClewError::new(
                    ErrorCode::SliceBudgetExceeded,
                    "documentation fact budget exceeded; select a smaller compilation",
                ));
            }
            let lease = store.read(&fact.payload, MAX_EVIDENCE as usize)?;
            let mut value: Value = serde_json::from_slice(lease.bytes()).map_err(io_error)?;
            // Attach explicit compilation-scope membership so a symbol that
            // appears under several scopes is retained per scope instead of
            // last-write-wins overwriting, including single-scope captures.
            value["scope"] = scope.clone();
            facts.push((value, fact.payload.digest.clone()));
            Ok(())
        })?;
    }
    if service.language == "kotlin" {
        facts = super::kotlin::project_facts(facts)?;
    }
    let wanted: BTreeSet<_> = facts
        .iter()
        .filter_map(|(f, _)| f["file"].as_str().map(str::to_owned))
        .collect();
    // Contract files are loaded once from the explicit original snapshot,
    // never flattened out of a per-compilation transformed table.
    let mut contracts = BTreeMap::new();
    for file in &service.contract_files {
        if let Some(text) = snapshot_text.get(file) {
            contracts.insert(file.clone(), text.to_string());
        }
    }
    let known: BTreeSet<String> = ready
        .compilations
        .iter()
        .map(|c| c.compilation.clone())
        .collect();
    let mut contents = BTreeMap::new();
    let mut transformed_map = BTreeMap::new();
    let wanted_snapshot_text: BTreeMap<String, Arc<str>> = snapshot_text
        .iter()
        .filter(|(path, _)| wanted.contains(*path))
        .map(|(path, text)| (path.clone(), Arc::clone(text)))
        .collect();
    let mut pool = SourcePool::new(MAX_EVIDENCE as usize);
    let snapshot_sources = if ready
        .compilations
        .iter()
        .any(|c| c.transformed_source.is_none())
    {
        Arc::new(
            wanted_snapshot_text
                .iter()
                .map(|(path, text)| Ok((path.clone(), pool.intern(Arc::clone(text))?)))
                .collect::<Result<SourceTable, ClewError>>()?,
        )
    } else {
        Arc::new(SourceTable::new())
    };
    for compilation in &ready.compilations {
        let scope = compilation.compilation.clone();
        let table = match &compilation.transformed_source {
            Some(reference) => {
                let bytes = generation_service::load_transformed_source(&store, reference)?;
                let mut table = BTreeMap::new();
                for path in &wanted {
                    if let Some(bytes) = bytes.get(path)
                        && let Ok(text) = std::str::from_utf8(bytes)
                    {
                        table.insert(path.clone(), pool.intern(Arc::from(text))?);
                    }
                }
                Arc::new(table)
            }
            // Scope and provenance remain distinct; equal source payloads and
            // their line indices are charged once, regardless of scope count.
            None => Arc::clone(&snapshot_sources),
        };
        contents.insert(scope.clone(), table);
        transformed_map.insert(scope, compilation.transformed_source.is_some());
    }
    let sources = CompilationSource {
        contents,
        transformed: transformed_map,
        contracts,
    };
    project_scoped(
        service,
        &session.base_revision,
        service_digest,
        &format!("{:?}", session.runtime_mode).to_uppercase(),
        &ready.coverage,
        facts,
        &sources,
        &known,
    )
}

pub fn source_id(service: &str, identity: &str) -> Result<String, ClewError> {
    Ok(format!("{}-{}", service, &digest(&identity)?[7..27]))
}
pub fn dependency_id(service: &str, kind: &str, identity: &str) -> Result<String, ClewError> {
    Ok(format!("{service}:{kind}:{}", &digest(&identity)?[7..31]))
}

pub fn source_link(
    service: &Service,
    revision: &str,
    file: &str,
    start: u64,
    end: u64,
) -> Option<String> {
    fn encode(s: &str) -> String {
        s.bytes()
            .map(|b| {
                if b.is_ascii_alphanumeric() || b"-._~/".contains(&b) {
                    (b as char).to_string()
                } else {
                    format!("%{b:02X}")
                }
            })
            .collect()
    }
    let template = service
        .source_link_template
        .as_deref()
        .unwrap_or("{repository}/blob/{revision}/{file}");
    Some(format!(
        "{}#L{start}-L{end}",
        template
            .replace("{repository}", &service.repository)
            .replace("{revision}", revision)
            .replace("{file}", &encode(file))
    ))
}

/// Per-compilation source text and transformed authority keyed by the canonical
/// compilation scope, plus an explicit original-snapshot contract table.
///
/// Occurrence (scope, path) stays distinct even when payloads are equal, so two
/// compilations sharing a relative path with different transformed text keep
/// both. Unscoped syntax projection uses the empty scope key.
pub(crate) struct CompilationSource {
    contents: BTreeMap<String, Arc<SourceTable>>,
    transformed: BTreeMap<String, bool>,
    contracts: BTreeMap<String, String>,
}

type SourceTable = BTreeMap<String, Arc<SourceBlob>>;

#[derive(Debug)]
struct SourceBlob {
    text: Arc<str>,
    line_ranges: Box<[Range<usize>]>,
}

/// Intern source payloads independently of compilation membership. The byte
/// limit measures retained text and line indices, not repeated scope references.
struct SourcePool {
    blobs: BTreeMap<String, Arc<SourceBlob>>,
    retained_bytes: usize,
    limit: usize,
}
impl SourcePool {
    fn new(limit: usize) -> Self {
        Self {
            blobs: BTreeMap::new(),
            retained_bytes: 0,
            limit,
        }
    }
    fn intern(&mut self, text: Arc<str>) -> Result<Arc<SourceBlob>, ClewError> {
        let key = canonical::hash_bytes(text.as_bytes());
        if let Some(blob) = self.blobs.get(&key) {
            if blob.text != text {
                return Err(ClewError::new(
                    ErrorCode::StateCorrupt,
                    "source payload hash collision",
                ));
            }
            return Ok(Arc::clone(blob));
        }
        let size = text.len().saturating_add(
            text.lines()
                .count()
                .saturating_mul(std::mem::size_of::<Range<usize>>()),
        );
        if size > self.limit.saturating_sub(self.retained_bytes) {
            return Err(ClewError::new(
                ErrorCode::SliceBudgetExceeded,
                "documentation unique source payload and line-index byte budget exceeded",
            ));
        }
        let blob = Arc::new(SourceBlob::new(text));
        self.retained_bytes += size;
        self.blobs.insert(key, Arc::clone(&blob));
        Ok(blob)
    }
}

impl SourceBlob {
    fn new(text: Arc<str>) -> Self {
        let bytes = text.as_bytes();
        let mut line_ranges = Vec::new();
        let mut start = 0;
        // Match `str::lines()` exactly: LF terminates a line, and a CR
        // immediately before LF is excluded from the line. A bare CR remains
        // ordinary source text rather than becoming a line separator.
        for index in 0..bytes.len() {
            if bytes[index] != b'\n' {
                continue;
            }
            let end = if index > start && bytes[index - 1] == b'\r' {
                index - 1
            } else {
                index
            };
            line_ranges.push(start..end);
            start = index + 1;
        }
        if start < bytes.len() {
            line_ranges.push(start..bytes.len());
        }
        Self {
            text,
            line_ranges: line_ranges.into_boxed_slice(),
        }
    }

    fn snippet(&self, start: u64, end: u64) -> Option<String> {
        if start == 0 || end < start || end > self.line_ranges.len() as u64 {
            return None;
        }
        Some(
            self.line_ranges[start as usize - 1..end as usize]
                .iter()
                .map(|range| &self.text[range.clone()])
                .collect::<Vec<_>>()
                .join("\n"),
        )
    }
}

fn source_table_from_text(text: &BTreeMap<String, Arc<str>>) -> Arc<SourceTable> {
    Arc::new(
        text.iter()
            .map(|(path, text)| (path.clone(), Arc::new(SourceBlob::new(Arc::clone(text)))))
            .collect(),
    )
}

impl CompilationSource {
    fn blob(&self, scope: &str, file: &str) -> Option<&SourceBlob> {
        self.contents.get(scope)?.get(file).map(Arc::as_ref)
    }
}

fn add_source(
    evidence: &mut ServiceEvidence,
    service: &Service,
    sources: &CompilationSource,
    scope: &str,
    fact: &Value,
    binding: &str,
    identity: &str,
) -> Result<Option<String>, ClewError> {
    let Some(file) = fact["file"].as_str() else {
        return Ok(None);
    };
    let transformed = sources.transformed.get(scope).copied().unwrap_or(false);
    let blob = sources.blob(scope, file);
    // Preserve the prior failure ordering: a transformed scope missing a
    // required file fails before coordinates are inspected.
    if blob.is_none() && !scope.is_empty() && transformed {
        return Err(invalid(format!(
            "transformed compilation {scope} lacks required source file {file}"
        )));
    }
    let (Some(start), Some(end)) = (fact["startLine"].as_u64(), fact["endLine"].as_u64()) else {
        return Ok(None);
    };
    let exact = if let Some(blob) = blob {
        blob.snippet(start, end)
    } else if scope.is_empty() {
        sources
            .contracts
            .get(file)
            .and_then(|text| SourceBlob::new(Arc::from(text.as_str())).snippet(start, end))
    } else {
        None
    };
    let Some(exact) = exact else {
        return Ok(None);
    };
    let id = source_id(&service.id, identity)?;
    // Transformed source coordinates refer to the persisted transformed bytes,
    // never the original commit snapshot. When transformed, identify the source
    // as such and omit the original-revision link unless the mapping is known.
    let (authority, url) = if transformed {
        (
            crate::generation_service::TRANSFORMED_SOURCE_AUTHORITY.into(),
            None,
        )
    } else {
        (
            "EXACT_SNAPSHOT_TEXT".into(),
            source_link(service, &evidence.revision, file, start, end),
        )
    };
    evidence.sources.insert(
        id.clone(),
        Source {
            id: id.clone(),
            service: service.id.clone(),
            revision: evidence.revision.clone(),
            file: file.into(),
            start_line: start,
            end_line: end,
            text_digest: canonical::hash_bytes(exact.as_bytes()),
            text: exact,
            evidence_digest: binding.into(),
            authority,
            occurrence: None,
            url,
        },
    );
    Ok(Some(id))
}

fn strip_coordinates(value: &Value) -> Value {
    match value {
        Value::Object(map) => Value::Object(
            map.iter()
                .filter(|(k, _)| {
                    !matches!(
                        k.as_str(),
                        "start"
                            | "end"
                            | "startLine"
                            | "endLine"
                            | "byteStart"
                            | "byteEnd"
                            | "line"
                            | "file"
                    )
                })
                .map(|(k, v)| (k.clone(), strip_coordinates(v)))
                .collect(),
        ),
        Value::Array(a) => Value::Array(a.iter().map(strip_coordinates).collect()),
        _ => value.clone(),
    }
}

/// Preserve lexical tokens, including string contents; ignore formatting/comments.
pub fn java_tokens(text: &str) -> Vec<String> {
    let chars: Vec<_> = text.chars().collect();
    let mut i = 0;
    let mut result = Vec::new();
    while i < chars.len() {
        if chars[i].is_whitespace() {
            i += 1;
            continue;
        }
        if chars[i] == '/' && chars.get(i + 1) == Some(&'/') {
            while i < chars.len() && chars[i] != '\n' {
                i += 1;
            }
            continue;
        }
        if chars[i] == '/' && chars.get(i + 1) == Some(&'*') {
            i += 2;
            while i + 1 < chars.len() && !(chars[i] == '*' && chars[i + 1] == '/') {
                i += 1;
            }
            i = (i + 2).min(chars.len());
            continue;
        }
        let start = i;
        if chars[i] == '"' || chars[i] == '\'' {
            let quote = chars[i];
            let block = quote == '"'
                && chars.get(i + 1) == Some(&quote)
                && chars.get(i + 2) == Some(&quote);
            i += if block { 3 } else { 1 };
            while i < chars.len() {
                if chars[i] == '\\' {
                    i = (i + 2).min(chars.len());
                    continue;
                }
                if chars[i] == quote
                    && (!block
                        || (chars.get(i + 1) == Some(&quote) && chars.get(i + 2) == Some(&quote)))
                {
                    i += if block { 3 } else { 1 };
                    break;
                }
                i += 1;
            }
        } else if chars[i].is_alphanumeric() || chars[i] == '_' || chars[i] == '$' {
            i += 1;
            while i < chars.len()
                && (chars[i].is_alphanumeric() || chars[i] == '_' || chars[i] == '$')
            {
                i += 1;
            }
        } else {
            i += 1;
        }
        result.push(chars[start..i].iter().collect());
    }
    result
}

/// Prepend a compilation scope to an evidence identity so the same symbol or
/// source path under different scopes is retained as a distinct observation
/// instead of colliding. Unscoped syntax facts retain their direct identity.
pub(super) fn scoped_identity(scope: &str, identity: &str) -> String {
    if scope.is_empty() {
        identity.to_string()
    } else {
        format!("{scope}\u{1f}{identity}")
    }
}

/// Resolve the canonical compilation-scope key from a fact's `scope` value.
///
/// Native capture always supplies an explicit registered compilation identity.
/// Only projection without a compilation model may omit scope.
fn resolve_scope_key(scope: &Value, known: &BTreeSet<String>) -> Result<String, ClewError> {
    match scope {
        Value::Object(map) => {
            let compilation = map
                .get("compilation")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .ok_or_else(|| invalid("fact scope object lacks a compilation identity"))?;
            if !known.contains(compilation) {
                return Err(invalid(format!(
                    "fact scope is not a registered compilation: {compilation}"
                )));
            }
            Ok(compilation.to_string())
        }
        Value::Null if known.is_empty() => Ok(String::new()),
        _ => Err(invalid(
            "fact requires an explicit registered compilation scope",
        )),
    }
}

#[allow(clippy::too_many_arguments)]
pub fn project(
    service: &Service,
    revision: &str,
    service_digest: &str,
    runtime_mode: &str,
    coverage: &str,
    facts: Vec<(Value, String)>,
    files: &BTreeMap<String, String>,
    transformed: bool,
) -> Result<ServiceEvidence, ClewError> {
    // This utility projects one shared source table. Native captures instead
    // use capture_session's independently admitted per-compilation tables.
    let known: BTreeSet<String> = if facts.iter().any(|(fact, _)| !fact["scope"].is_null()) {
        service.effective_compilations().into_iter().collect()
    } else {
        BTreeSet::new()
    };
    let scopes = if known.is_empty() {
        BTreeSet::from([String::new()])
    } else {
        known.clone()
    };
    let table = source_table_from_text(
        &files
            .iter()
            .map(|(path, text)| (path.clone(), Arc::from(text.as_str())))
            .collect(),
    );
    let contents = scopes
        .iter()
        .map(|scope| (scope.clone(), Arc::clone(&table)))
        .collect();
    let transformed_map = scopes
        .into_iter()
        .map(|scope| (scope, transformed))
        .collect();
    let sources = CompilationSource {
        contents,
        transformed: transformed_map,
        contracts: files.clone(),
    };
    project_scoped(
        service,
        revision,
        service_digest,
        runtime_mode,
        coverage,
        facts,
        &sources,
        &known,
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn project_scoped(
    service: &Service,
    revision: &str,
    service_digest: &str,
    runtime_mode: &str,
    coverage: &str,
    facts: Vec<(Value, String)>,
    sources: &CompilationSource,
    known: &BTreeSet<String>,
) -> Result<ServiceEvidence, ClewError> {
    let mut evidence = ServiceEvidence {
        schema: "codeclew-documentation-service-evidence/1.0".into(),
        service: service.id.clone(),
        revision: revision.into(),
        service_digest: service_digest.into(),
        extractor: EXTRACTOR.into(),
        runtime_mode: runtime_mode.into(),
        coverage: coverage.into(),
        boundaries: vec![],
        entrypoints: vec![],
        observations: BTreeMap::new(),
        sources: BTreeMap::new(),
        contracts: BTreeMap::new(),
    };
    let annotation_registry = spring_entrypoints::annotation_registry(facts.iter().map(|(f, _)| f));
    // Track, per symbol, the distinct normalized payload digests observed
    // across admitted compilation scopes. A symbol that resolves to multiple
    // incompatible candidates becomes an explicit SCOPE_AMBIGUOUS boundary
    // rather than last-write-wins overwriting a single candidate.
    let mut symbol_digests: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for (fact, binding) in &facts {
        if fact["kind"] == "BOUNDARY" {
            evidence.boundaries.push(
                fact["code"]
                    .as_str()
                    .unwrap_or("JVM_ANALYSIS_BOUNDARY")
                    .into(),
            );
            continue;
        }
        if fact["kind"] != "DECLARATION" {
            continue;
        }
        let symbol = fact["symbolIdentity"]
            .as_str()
            .ok_or_else(|| invalid("compiler declaration lacks identity"))?;
        let scope = resolve_scope_key(&fact["scope"], known)?;
        let scoped = scoped_identity(&scope, symbol);
        let id = dependency_id(&service.id, "symbol", &scoped)?;
        let source = add_source(
            &mut evidence,
            service,
            sources,
            &scope,
            fact,
            binding,
            &scoped,
        )?;
        let mut normalized = strip_coordinates(fact);
        if let Some(s) = source.as_ref().and_then(|id| evidence.sources.get(id)) {
            normalized["sourceTokens"] = json!(java_tokens(&s.text));
        }
        // Ambiguity is judged on symbol content independent of which scope it
        // appeared in: identical payloads across scopes are not ambiguous,
        // only incompatible candidates are. strip_coordinates preserves the
        // injected `scope` key, so drop it from the content digest.
        let mut content = normalized.clone();
        if let Value::Object(map) = &mut content {
            map.remove("scope");
        }
        let content_digest = digest(&content)?;
        if !scope.is_empty() {
            normalized["scope"] = json!(scope);
        }
        let obs_digest = digest(&normalized)?;
        symbol_digests
            .entry(symbol.to_string())
            .or_default()
            .insert(content_digest);
        let source_ids: Vec<_> = source.into_iter().collect();
        let observation = Observation {
            id: id.clone(),
            kind: "SYMBOL".into(),
            service: service.id.clone(),
            symbol: symbol.into(),
            digest: obs_digest,
            normalized,
            source_ids: source_ids.clone(),
        };
        evidence.observations.insert(id.clone(), observation);
        if let Some(flow) = fact.get("documentation") {
            let events = flow["events"]
                .as_array()
                .ok_or_else(|| invalid("compiler flow has no events"))?;
            if let Some(boundaries) = flow["boundaries"].as_array() {
                evidence.boundaries.extend(
                    boundaries
                        .iter()
                        .filter_map(Value::as_str)
                        .map(str::to_owned),
                );
            }
            for (index, event) in events.iter().enumerate() {
                let identity = scoped_identity(&scope, &format!("{symbol}/event/{index}"));
                let event_id = dependency_id(&service.id, "flow", &identity)?;
                let event_source = add_source(
                    &mut evidence,
                    service,
                    sources,
                    &scope,
                    event,
                    binding,
                    &identity,
                )?;
                let mut normalized = strip_coordinates(event);
                normalized["ordinal"] = json!(index);
                if !scope.is_empty() {
                    normalized["scope"] = json!(scope);
                }
                evidence.observations.insert(
                    event_id.clone(),
                    Observation {
                        id: event_id,
                        kind: "FLOW".into(),
                        service: service.id.clone(),
                        symbol: symbol.into(),
                        digest: digest(&normalized)?,
                        normalized,
                        source_ids: event_source.into_iter().collect(),
                    },
                );
            }
        }
        let spring_fact = spring_entrypoints::with_annotation_registry(fact, &annotation_registry)?;
        if let Some(metadata) = spring_entrypoints::metadata_for_fact(
            &spring_fact,
            if service.language == "kotlin" {
                "K2_RESOLVED_ANNOTATIONS"
            } else {
                "JAVAC_RESOLVED_ANNOTATIONS"
            },
        )? {
            for (ordinal, entry) in metadata.entries.iter().enumerate() {
                let target = entry.target_symbol.as_deref().unwrap_or(symbol);
                let identity = if entry.target_symbol.is_some() {
                    format!(
                        "{target}/bean:{}/{ordinal}",
                        entry.bean_class.as_deref().unwrap_or(symbol)
                    )
                } else {
                    format!("{target}/{ordinal}")
                };
                let eid = source_id(
                    &service.id,
                    &scoped_identity(&scope, &format!("entrypoint/{identity}")),
                )?;
                let trigger = spring_entrypoints::describe_trigger(entry);
                let route_id = dependency_id(
                    &service.id,
                    "entrypoint",
                    &scoped_identity(&scope, &identity),
                )?;
                let mut normalized = json!({"trigger":trigger,"binding":entry,"boundaries":metadata.boundaries,"frameworkDerivation":metadata.derivation});
                if !scope.is_empty() {
                    normalized["scope"] = json!(scope);
                }
                evidence.observations.insert(
                    route_id.clone(),
                    Observation {
                        id: route_id.clone(),
                        kind: "ENTRYPOINT".into(),
                        service: service.id.clone(),
                        symbol: target.into(),
                        digest: digest(&normalized)?,
                        normalized,
                        source_ids: source_ids.clone(),
                    },
                );
                evidence.entrypoints.push(Entrypoint {
                    id: eid,
                    service: service.id.clone(),
                    symbol: target.into(),
                    kind: entry.kind.clone(),
                    trigger,
                    source_ids: source_ids.clone(),
                    dependency_ids: vec![id.clone(), route_id],
                    boundaries: metadata.boundaries.clone(),
                });
            }
        }
    }
    // A symbol admitted under multiple compilation scopes with incompatible
    // candidate payloads is a visible SCOPE_AMBIGUOUS boundary, not a silent
    // last-write-wins selection. Identical payloads across scopes are not
    // ambiguous (their observations coexist under distinct scope identities).
    for (symbol, digests) in &symbol_digests {
        if digests.len() > 1 {
            evidence
                .boundaries
                .push(format!("SCOPE_AMBIGUOUS:{symbol}"));
        }
    }
    super::contracts::import(service, &mut evidence, &sources.contracts)?;
    for entry in &mut evidence.entrypoints {
        if !evidence.observations.values().any(|o| {
            o.kind == "SYMBOL"
                && o.symbol == entry.symbol
                && o.normalized.get("documentation").is_some()
        }) {
            entry
                .boundaries
                .push("IMPLEMENTATION_BODY_UNAVAILABLE".into());
        }
    }
    evidence.entrypoints.sort_by(|a, b| a.id.cmp(&b.id));
    evidence.entrypoints.dedup_by(|a, b| a.id == b.id);
    evidence.boundaries.sort();
    evidence.boundaries.dedup();
    super::contracts::enrich(&mut evidence)?;
    verify_evidence(&evidence)?;
    Ok(evidence)
}

pub fn verify_evidence(e: &ServiceEvidence) -> Result<(), ClewError> {
    if e.schema != "codeclew-documentation-service-evidence/1.0"
        || e.revision.len() != 40
        || !e.revision.bytes().all(|b| b.is_ascii_hexdigit())
        || ![EXTRACTOR, SOURCE_EXTRACTOR].contains(&e.extractor.as_str())
    {
        return Err(invalid("portable evidence version or revision is invalid"));
    }
    for s in e.sources.values() {
        if s.service != e.service
            || s.revision != e.revision
            || s.start_line == 0
            || s.end_line < s.start_line
            || s.end_line - s.start_line + 1 != s.text.lines().count() as u64
            || canonical::hash_bytes(s.text.as_bytes()) != s.text_digest
        {
            return Err(invalid("portable source binding is inconsistent"));
        }
        if let Some(occurrence) = &s.occurrence
            && (occurrence.end_byte.checked_sub(occurrence.start_byte) != Some(s.text.len())
                || s.evidence_digest
                    != digest(&(
                        SOURCE_EXTRACTOR,
                        &occurrence.snapshot,
                        &occurrence.blob,
                        occurrence.start_byte,
                        occurrence.end_byte,
                    ))?)
        {
            return Err(invalid("portable byte occurrence is inconsistent"));
        }
        store::relative(&s.file)?;
    }
    for o in e.observations.values() {
        if digest(&o.normalized)? != o.digest
            || o.source_ids.iter().any(|id| !e.sources.contains_key(id))
        {
            return Err(invalid(
                "portable observation digest or source references are inconsistent",
            ));
        }
    }
    Ok(())
}

fn descriptor_parameters(descriptor: &str) -> Vec<String> {
    let Some(parameters) = descriptor
        .strip_prefix('(')
        .and_then(|s| s.split_once(')').map(|(p, _)| p))
    else {
        return vec![];
    };
    let mut input = parameters.chars().peekable();
    let mut output = Vec::new();
    while input.peek().is_some() {
        let mut dimensions = 0;
        while input.peek() == Some(&'[') {
            input.next();
            dimensions += 1;
        }
        let value = match input.next() {
            Some('B') => "byte".into(),
            Some('C') => "char".into(),
            Some('D') => "double".into(),
            Some('F') => "float".into(),
            Some('I') => "int".into(),
            Some('J') => "long".into(),
            Some('S') => "short".into(),
            Some('Z') => "boolean".into(),
            Some('L') => input
                .by_ref()
                .take_while(|c| *c != ';')
                .collect::<String>()
                .replace(['/', '$'], "."),
            _ => return vec![],
        };
        output.push(format!("{}{}", value, "[]".repeat(dimensions)));
    }
    output
}

pub fn resolve<'a>(
    selector: Option<&Selector>,
    evidence: &'a ServiceEvidence,
) -> Vec<&'a Observation> {
    let Some(selector) = selector else {
        return vec![];
    };
    let owner = if selector.owner.starts_with("class:") || selector.owner.starts_with("package:") {
        selector.owner.clone()
    } else {
        format!("class:{}", selector.owner)
    };
    evidence
        .observations
        .values()
        .filter(|o| {
            o.kind == "SYMBOL"
                && selector
                    .scope
                    .as_ref()
                    .is_none_or(|scope| o.normalized["scope"].as_str().unwrap_or("") == scope)
                && o.normalized["ownerIdentity"]
                    .as_str()
                    .is_some_and(|observed| observed.replace('/', ".") == owner)
                && o.normalized["name"] == selector.name
                && selector.parameter_types.as_ref().is_none_or(|parameters| {
                    o.normalized
                        .pointer("/documentation/parameterTypes")
                        .cloned()
                        .unwrap_or_else(|| {
                            json!(descriptor_parameters(
                                o.normalized["jvmDescriptor"].as_str().unwrap_or("")
                            ))
                        })
                        == json!(parameters)
                })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn source_normalization_preserves_strings_and_identifiers() {
        assert_eq!(
            java_tokens("return /* shifted */ call ( a );"),
            java_tokens("return call(a);")
        );
        assert_ne!(java_tokens("a b"), java_tokens("ab"));
        assert_ne!(java_tokens("\"a b\""), java_tokens("\"ab\""));
        assert_eq!(java_tokens("// moved\nreturn x;"), java_tokens("return x;"));
    }

    #[test]
    fn scope_key_resolves_production_object_and_rejects_malformed() {
        let known: BTreeSet<String> = [
            ":common/main".into(),
            ":flows:lead-tinkoff-decision-flow/main".into(),
        ]
        .into_iter()
        .collect();
        // The explicit compilation field yields a distinct stable key.
        let object_scope = json!({ "compilation": ":flows:lead-tinkoff-decision-flow/main" });
        assert_eq!(
            resolve_scope_key(&object_scope, &known).unwrap(),
            ":flows:lead-tinkoff-decision-flow/main"
        );
        assert!(resolve_scope_key(&json!(":common/main"), &known).is_err());
        assert!(resolve_scope_key(&Value::Null, &known).is_err());
        assert!(resolve_scope_key(&json!(42), &known).is_err());
        assert_eq!(
            resolve_scope_key(&Value::Null, &BTreeSet::new()).unwrap(),
            ""
        );
        // A malformed object without a compilation identity is an explicit error.
        let malformed = resolve_scope_key(&json!({"runtime_key": "x"}), &known).unwrap_err();
        assert!(
            malformed.message.contains("compilation identity"),
            "{malformed}"
        );
        // An unknown registered scope is an explicit error, never empty fallback.
        let unknown =
            resolve_scope_key(&json!({"compilation": ":unknown/main"}), &known).unwrap_err();
        assert!(
            unknown.message.contains("not a registered compilation"),
            "{unknown}"
        );
    }
    #[test]
    fn locators_do_not_export_credentials() {
        assert_eq!(
            locator("ssh://git@example.invalid:2222/team/service.git"),
            locator("https://example.invalid/team/service")
        );
        assert!(locator("ssh://git@example.invalid:invalid/team/service.git").is_none());

        assert_eq!(
            locator("git@example.invalid:team/service.git"),
            locator("https://example.invalid/team/service")
        );
        assert!(
            locator(&format!(
                "https://{}@example.invalid/service",
                "user:fixture-password"
            ))
            .is_none()
        );
    }

    #[test]
    fn project_marks_transformed_sources_and_omits_original_links() {
        let service: Service = serde_json::from_value(json!({
            "schema":"codeclew-documentation-service/1.0", "id":"svc", "title":"S",
            "repositoryId":"svc", "repository":"https://example.invalid/svc",
            "language":"java", "profile":"java-17plus-maven-writable-then-seal",
            "compilations":[":/main"], "targetRef":"main",
            "sourceLinkTemplate":"{repository}/blob/{revision}/{file}",
        }))
        .unwrap();
        let text = "package example;\npublic class Service { void m() {} }\n";
        let file = "src/main/java/example/Service.java";
        let declaration = json!({
            "kind":"DECLARATION","symbolIdentity":"example.Service",
            "ownerIdentity":"example","name":"Service","file":file,
            "startLine":2,"endLine":2,"resolution":"RESOLVED",
            "documentation":{"events":[]},
        });
        let facts = vec![(declaration, "binding-digest".into())];
        let files = BTreeMap::from([(file.into(), text.into())]);

        // Transformed: source labelled TRANSFORMED_SOURCE, no original URL.
        let transformed = project(
            &service,
            &"a".repeat(40),
            &digest(&service).unwrap(),
            "DEVELOPMENT",
            "PARTIAL",
            facts.clone(),
            &files,
            true,
        )
        .unwrap();
        let source = transformed.sources.values().next().unwrap();
        assert_eq!(source.authority, "TRANSFORMED_SOURCE");
        assert_eq!(
            source.url, None,
            "transformed source must not claim an original-commit link"
        );
        assert_eq!(source.text, "public class Service { void m() {} }");

        // Read-only: EXACT_SNAPSHOT_TEXT with an original-commit URL.
        let read_only = project(
            &service,
            &"a".repeat(40),
            &digest(&service).unwrap(),
            "DEVELOPMENT",
            "PARTIAL",
            facts,
            &files,
            false,
        )
        .unwrap();
        let source = read_only.sources.values().next().unwrap();
        assert_eq!(source.authority, "EXACT_SNAPSHOT_TEXT");
        assert!(
            source
                .url
                .as_deref()
                .is_some_and(|url| url.contains("/blob/") && url.contains("#L2-L2")),
            "read-only source must retain an original-commit link"
        );
    }

    fn projection_service() -> Service {
        serde_json::from_value(json!({
            "schema":"codeclew-documentation-service/1.0", "id":"svc", "title":"S",
            "repositoryId":"svc", "repository":"https://example.invalid/svc",
            "language":"java", "profile":"java-17plus-maven-writable-then-seal",
            "compilations":[":/main"], "targetRef":"main",
            "sourceLinkTemplate":"{repository}/blob/{revision}/{file}",
            "contractFiles":[],
        }))
        .unwrap()
    }

    fn declaration_fact(scope: &Value, symbol: &str, file: &str, line: u64) -> (Value, String) {
        (
            json!({
                "kind":"DECLARATION","symbolIdentity":symbol,"ownerIdentity":"example",
                "name":symbol.rsplit('.').next().unwrap_or(symbol),
                "file":file,"startLine":line,"endLine":line,"resolution":"RESOLVED",
                "documentation":{"events":[]},"scope":scope,
            }),
            "binding-digest".into(),
        )
    }

    fn compile_sources(
        scopes: Vec<(&str, BTreeMap<String, String>, bool)>,
        contracts: BTreeMap<String, String>,
    ) -> CompilationSource {
        let mut contents = BTreeMap::new();
        let mut transformed = BTreeMap::new();
        for (scope, table, is_transformed) in scopes {
            contents.insert(
                scope.to_string(),
                source_table_from_text(
                    &table
                        .iter()
                        .map(|(path, text)| (path.clone(), Arc::from(text.as_str())))
                        .collect(),
                ),
            );
            transformed.insert(scope.to_string(), is_transformed);
        }
        CompilationSource {
            contents,
            transformed,
            contracts,
        }
    }

    #[test]
    fn source_blob_shares_original_text_and_preserves_line_semantics() {
        let file = "src/main/java/example/Shared.java";
        let blob = Arc::new(SourceBlob::new(Arc::from("first\r\nsecond\r\n")));
        let table = Arc::new(BTreeMap::from([(file.to_string(), Arc::clone(&blob))]));
        let sources = CompilationSource {
            contents: BTreeMap::from([
                (":a/main".into(), Arc::clone(&table)),
                (":b/main".into(), Arc::clone(&table)),
            ]),
            transformed: BTreeMap::from([(":a/main".into(), false), (":b/main".into(), false)]),
            contracts: BTreeMap::new(),
        };

        assert!(Arc::ptr_eq(
            sources.contents.get(":a/main").unwrap(),
            sources.contents.get(":b/main").unwrap()
        ));
        assert!(Arc::ptr_eq(
            sources.contents.get(":a/main").unwrap().get(file).unwrap(),
            sources.contents.get(":b/main").unwrap().get(file).unwrap()
        ));
        // This is the existing `text.lines().collect().join("\\n")` contract:
        // CRLF separators and a trailing newline are not copied into evidence.
        assert_eq!(blob.snippet(1, 2).as_deref(), Some("first\nsecond"));
        assert_eq!(blob.snippet(2, 2).as_deref(), Some("second"));
        assert_eq!(blob.snippet(3, 3), None);
    }

    #[test]
    fn source_pool_charges_shared_payload_once_across_128_scopes() {
        let text: Arc<str> = Arc::from("first\nsecond\n");
        let required = text.len() + 2 * std::mem::size_of::<Range<usize>>();
        let mut pool = SourcePool::new(required);
        let first = pool.intern(Arc::clone(&text)).unwrap();
        for _ in 0..128 {
            assert!(Arc::ptr_eq(
                &first,
                &pool.intern(Arc::from(text.as_ref())).unwrap()
            ));
        }
        assert_eq!(pool.retained_bytes, required);
        assert_eq!(first.snippet(1, 2).as_deref(), Some("first\nsecond"));
        assert!(pool.intern(Arc::from("changed")).is_err());
        assert_eq!(
            pool.retained_bytes, required,
            "rejected payload must not consume budget"
        );
    }

    #[test]
    fn source_pool_preserves_distinct_transformed_bytes_and_counts_line_indices() {
        let mut pool = SourcePool::new(1024);
        let first = pool.intern(Arc::from("first\n")).unwrap();
        let second = pool.intern(Arc::from("second\n")).unwrap();
        assert!(!Arc::ptr_eq(&first, &second));
        assert_eq!(first.snippet(1, 1).as_deref(), Some("first"));
        assert_eq!(second.snippet(1, 1).as_deref(), Some("second"));
        assert_eq!(
            pool.retained_bytes,
            13 + 2 * std::mem::size_of::<Range<usize>>()
        );
        let mut lines = SourcePool::new(10);
        assert!(lines.intern(Arc::from("\n\n")).is_err());
    }

    #[test]
    fn source_blob_ranges_differentially_match_str_lines() {
        let samples = [
            "",
            "a",
            "\n",
            "a\n",
            "a\r\nb",
            "a\r\n",
            "a\rb",
            "a\r\nb\r",
            "α\r\nβ\nγ",
            "\r\n\r\n",
        ];
        for text in samples {
            let blob = SourceBlob::new(Arc::from(text));
            let lines = text.lines().collect::<Vec<_>>();
            for start in 0..=(lines.len() as u64 + 2) {
                for end in 0..=(lines.len() as u64 + 2) {
                    let expected = if start == 0 || end < start || end > lines.len() as u64 {
                        None
                    } else {
                        Some(lines[start as usize - 1..end as usize].join("\n"))
                    };
                    assert_eq!(
                        blob.snippet(start, end),
                        expected,
                        "text={text:?} {start}..{end}"
                    );
                }
            }
        }
    }

    fn known(scopes: &[&str]) -> BTreeSet<String> {
        scopes.iter().map(|s| (*s).to_string()).collect()
    }

    fn project_scoped_ok(
        service: &Service,
        facts: Vec<(Value, String)>,
        sources: &CompilationSource,
        scopes: &[&str],
    ) -> ServiceEvidence {
        project_scoped(
            service,
            &"a".repeat(40),
            &digest(service).unwrap(),
            "DEVELOPMENT",
            "PARTIAL",
            facts,
            sources,
            &known(scopes),
        )
        .unwrap()
    }

    #[test]
    fn scoped_sources_same_path_different_bytes_produce_separate_records() {
        let service = projection_service();
        let file = "src/main/java/example/Service.java";
        let text_a = "package example;\npublic class Service { void a() {} }\n";
        let text_b = "package example;\npublic class Service { void b() {} }\n";
        let mut table_a = BTreeMap::new();
        table_a.insert(file.into(), text_a.into());
        let mut table_b = BTreeMap::new();
        table_b.insert(file.into(), text_b.into());
        let sources = compile_sources(
            vec![(":a/main", table_a, true), (":b/main", table_b, true)],
            BTreeMap::new(),
        );
        let facts = vec![
            declaration_fact(
                &json!({"compilation":":a/main"}),
                "example.Service",
                file,
                2,
            ),
            declaration_fact(
                &json!({"compilation":":b/main"}),
                "example.Service",
                file,
                2,
            ),
        ];
        let evidence = project_scoped_ok(&service, facts, &sources, &[":a/main", ":b/main"]);
        assert_eq!(evidence.sources.len(), 2, "one source record per scope");
        let texts: BTreeSet<_> = evidence.sources.values().map(|s| s.text.clone()).collect();
        assert_eq!(
            texts,
            BTreeSet::from([
                "public class Service { void a() {} }".to_string(),
                "public class Service { void b() {} }".to_string(),
            ]),
            "different transformed bytes per scope must survive as separate records"
        );
        for source in evidence.sources.values() {
            assert_eq!(source.start_line, 2);
            assert_eq!(source.end_line, 2);
            assert_eq!(source.authority, "TRANSFORMED_SOURCE");
        }
    }

    #[test]
    fn mixed_readonly_and_transformed_scopes_choose_independent_authority() {
        let service = projection_service();
        let file = "src/main/java/example/Service.java";
        let table = BTreeMap::from([(
            file.into(),
            "package example;\npublic class Service {}\n".into(),
        )]);
        let sources = compile_sources(
            vec![
                (":writable/main", table.clone(), true),
                (":readonly/main", table.clone(), false),
            ],
            BTreeMap::new(),
        );
        let facts = vec![
            declaration_fact(
                &json!({"compilation":":writable/main"}),
                "example.Service",
                file,
                2,
            ),
            declaration_fact(
                &json!({"compilation":":readonly/main"}),
                "example.Service",
                file,
                2,
            ),
        ];
        let evidence = project_scoped_ok(
            &service,
            facts,
            &sources,
            &[":writable/main", ":readonly/main"],
        );
        assert_eq!(evidence.sources.len(), 2, "one source per scope");
        let writable = evidence
            .sources
            .values()
            .find(|s| s.authority == "TRANSFORMED_SOURCE")
            .unwrap();
        assert_eq!(
            writable.url, None,
            "writable must omit original-commit link"
        );
        let readonly = evidence
            .sources
            .values()
            .find(|s| s.authority == "EXACT_SNAPSHOT_TEXT")
            .unwrap();
        assert!(
            readonly
                .url
                .as_deref()
                .is_some_and(|u| u.contains("/blob/")),
            "readonly must retain original-commit link"
        );
    }

    #[test]
    fn scoped_projection_rejects_malformed_and_unregistered_scope() {
        let service = projection_service();
        let file = "src/main/java/example/Service.java";
        let mut table = BTreeMap::new();
        table.insert(
            file.into(),
            "package example;\npublic class Service {}\n".into(),
        );
        let sources = compile_sources(vec![(":a/main", table, true)], BTreeMap::new());
        // Missing transformed source fails before coordinate validation, as it
        // did before source sharing was introduced.
        let missing_source = project_scoped(
            &service,
            &"a".repeat(40),
            &digest(&service).unwrap(),
            "DEVELOPMENT",
            "PARTIAL",
            vec![(
                json!({
                    "kind":"DECLARATION","symbolIdentity":"example.Missing",
                    "ownerIdentity":"example","name":"Missing",
                    "file":"src/main/java/example/Missing.java",
                    "scope":{"compilation":":a/main"},
                }),
                "binding-digest".into(),
            )],
            &sources,
            &known(&[":a/main"]),
        )
        .unwrap_err();
        assert!(
            missing_source
                .message
                .contains("lacks required source file")
        );

        // Malformed object scope (no compilation identity) is an explicit error.
        let malformed = project_scoped(
            &service,
            &"a".repeat(40),
            &digest(&service).unwrap(),
            "DEVELOPMENT",
            "PARTIAL",
            vec![declaration_fact(
                &json!({"runtime_key":"x"}),
                "example.Service",
                file,
                2,
            )],
            &sources,
            &known(&[":a/main"]),
        )
        .unwrap_err();
        assert!(
            malformed.message.contains("compilation identity"),
            "{malformed}"
        );
        // An unregistered compilation scope is an explicit error, never a silent
        // fallback to the empty or another compilation's table.
        let unregistered = project_scoped(
            &service,
            &"a".repeat(40),
            &digest(&service).unwrap(),
            "DEVELOPMENT",
            "PARTIAL",
            vec![declaration_fact(
                &json!({"compilation":":unknown/main"}),
                "example.Service",
                file,
                2,
            )],
            &sources,
            &known(&[":a/main"]),
        )
        .unwrap_err();
        assert!(
            unregistered
                .message
                .contains("not a registered compilation"),
            "{unregistered}"
        );
    }

    #[test]
    fn single_compilation_preserves_explicit_scope() {
        let service = projection_service();
        let file = "src/main/java/example/Service.java";
        let mut table = BTreeMap::new();
        table.insert(
            file.into(),
            "package example;\npublic class Service {}\n".into(),
        );
        let sources = compile_sources(vec![(":/main", table, true)], BTreeMap::new());
        let evidence = project_scoped_ok(
            &service,
            vec![declaration_fact(
                &json!({"compilation": ":/main"}),
                "example.Service",
                file,
                2,
            )],
            &sources,
            &[":/main"],
        );
        assert_eq!(evidence.sources.len(), 1);
        let source = evidence.sources.values().next().unwrap();
        assert_eq!(source.text, "public class Service {}");
        assert_eq!(source.authority, "TRANSFORMED_SOURCE");
    }

    #[test]
    fn identical_shared_declarations_across_scopes_are_not_ambiguous() {
        let service = projection_service();
        let file = "src/main/java/example/Shared.java";
        let text = "package example;\npublic class Shared {}\n";
        let mut table = BTreeMap::new();
        table.insert(file.into(), text.into());
        // Same symbol and identical payloads under two scopes must coexist
        // without a SCOPE_AMBIGUOUS boundary.
        let sources = compile_sources(
            vec![(":a/main", table.clone(), true), (":b/main", table, true)],
            BTreeMap::new(),
        );
        let facts = vec![
            declaration_fact(&json!({"compilation":":a/main"}), "example.Shared", file, 2),
            declaration_fact(&json!({"compilation":":b/main"}), "example.Shared", file, 2),
        ];
        let evidence = project_scoped_ok(&service, facts, &sources, &[":a/main", ":b/main"]);
        assert_eq!(evidence.sources.len(), 2, "both scope records retained");
        assert_eq!(
            evidence
                .observations
                .values()
                .filter(|o| o.kind == "SYMBOL" && o.symbol == "example.Shared")
                .count(),
            2
        );
        assert!(
            !evidence
                .boundaries
                .iter()
                .any(|b| b.starts_with("SCOPE_AMBIGUOUS")),
            "identical payloads across scopes are not ambiguous: {:?}",
            evidence.boundaries
        );
    }

    #[test]
    fn contract_files_load_independently_of_transformed_java_table() {
        let service = serde_json::from_value(json!({
            "schema":"codeclew-documentation-service/1.0", "id":"svc", "title":"S",
            "repositoryId":"svc", "repository":"https://example.invalid/svc",
            "language":"java", "profile":"java-17plus-maven-writable-then-seal",
            "compilations":[":/main"], "targetRef":"main",
            "sourceLinkTemplate":"{repository}/blob/{revision}/{file}",
            "contractFiles":["api/openapi.yaml"],
        }))
        .unwrap();
        // The transformed Java table intentionally has no contract path; the
        // contract must still load from the explicit contract table and never
        // be flattened out of a per-compilation transformed map.
        let mut table = BTreeMap::new();
        table.insert(
            "src/main/java/example/Service.java".into(),
            "package example;\npublic class Service {}\n".into(),
        );
        let sources = compile_sources(
            vec![(":a/main", table, true)],
            BTreeMap::from([(
                "api/openapi.yaml".into(),
                "openapi: 3.0.3\ninfo: {title: S, version: \"1\"}\npaths: {}\n".into(),
            )]),
        );
        let evidence = project_scoped_ok(
            &service,
            vec![declaration_fact(
                &json!({"compilation":":a/main"}),
                "example.Service",
                "src/main/java/example/Service.java",
                2,
            )],
            &sources,
            &[":a/main"],
        );
        assert!(
            evidence.contracts.contains_key("api/openapi.yaml"),
            "contract must load from its explicit snapshot table"
        );
        assert!(
            evidence
                .observations
                .values()
                .any(|o| o.kind == "CONTRACT" && o.symbol == "api/openapi.yaml"),
            "contract observation must be registered"
        );
        // Both the Java declaration source and the declared contract source are
        // bound independently; the contract path is not part of the Java table.
        assert_eq!(evidence.sources.len(), 2);
        assert!(
            evidence
                .sources
                .values()
                .any(|s| s.file == "src/main/java/example/Service.java"),
            "Java source must be bound"
        );
        assert!(
            evidence
                .sources
                .values()
                .any(|s| s.file == "api/openapi.yaml"),
            "contract source must be bound"
        );
    }
}
