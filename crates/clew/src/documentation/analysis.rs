//! Current JVM evidence uses the normal admitted session and immutable generation.
use super::{
    bytes, digest, invalid, io_error,
    local_cfg::{
        LOCAL_CFG_EVIDENCE_SCHEMA, LocalCfgBoundaryEvidence, LocalCfgEvidence,
        LocalCfgNodeCitation, LocalCfgSourceSite,
    },
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
            super::syntax_file_cache::producer_identity(),
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
            super::syntax::capture_cached(service, &repo, repository)
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
    let language = match service.language.as_str() {
        "kotlin" => SessionLanguage::Kotlin,
        "csharp" => SessionLanguage::CSharp,
        _ => SessionLanguage::Java,
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
            let domain = match service.language.as_str() {
                "kotlin" => "analysis:kotlin-semantic-facts",
                "csharp" => super::csharp::CSHARP_FACTS_DOMAIN,
                _ => "analysis:java-compiler-facts",
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
    match service.language.as_str() {
        "kotlin" => facts = super::kotlin::project_facts(facts)?,
        "csharp" => facts = super::csharp::project_facts(facts)?,
        _ => {}
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
    content_digest: String,
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
            content_digest: canonical::hash_bytes(text.as_bytes()),
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

fn add_kotlin_call_site_source(
    evidence: &mut ServiceEvidence,
    service: &Service,
    sources: &CompilationSource,
    scope: &str,
    fact: &Value,
    binding: &str,
    identity: &str,
) -> Result<Option<(String, Value)>, ClewError> {
    let file = fact["file"]
        .as_str()
        .ok_or_else(|| invalid("Kotlin CALLS relation lacks its source file"))?;
    let start = fact["byteStart"]
        .as_u64()
        .ok_or_else(|| invalid("Kotlin CALLS relation lacks its byte start"))?;
    let end = fact["byteEnd"]
        .as_u64()
        .ok_or_else(|| invalid("Kotlin CALLS relation lacks its byte end"))?;
    if start >= end {
        return Err(invalid(
            "Kotlin CALLS relation has an empty or reversed source range",
        ));
    }
    let Some(blob) = sources.blob(scope, file) else {
        return Ok(None);
    };
    let start = usize::try_from(start)
        .map_err(|_| invalid("Kotlin CALLS source start exceeds addressable bytes"))?;
    let end = usize::try_from(end)
        .map_err(|_| invalid("Kotlin CALLS source end exceeds addressable bytes"))?;
    if end > blob.text.len()
        || !blob.text.is_char_boundary(start)
        || !blob.text.is_char_boundary(end)
    {
        return Err(invalid(
            "Kotlin CALLS relation source range is outside retained UTF-8 boundaries",
        ));
    }
    let text = &blob.text[start..end];
    let start_line = 1 + blob.text.as_bytes()[..start]
        .iter()
        .filter(|byte| **byte == b'\n')
        .count() as u64;
    let end_line = 1 + blob.text.as_bytes()[..end - 1]
        .iter()
        .filter(|byte| **byte == b'\n')
        .count() as u64;
    let source_id = source_id(&service.id, identity)?;
    let transformed = sources.transformed.get(scope).copied().unwrap_or(false);
    let (authority, url) = if transformed {
        (
            crate::generation_service::TRANSFORMED_SOURCE_AUTHORITY.into(),
            None,
        )
    } else {
        (
            "EXACT_SNAPSHOT_TEXT".into(),
            source_link(service, &evidence.revision, file, start_line, end_line),
        )
    };
    let text_digest = canonical::hash_bytes(text.as_bytes());
    evidence.sources.insert(
        source_id.clone(),
        Source {
            id: source_id.clone(),
            service: service.id.clone(),
            revision: evidence.revision.clone(),
            file: file.into(),
            start_line,
            end_line,
            text: text.into(),
            text_digest: text_digest.clone(),
            evidence_digest: binding.into(),
            authority,
            occurrence: None,
            url,
        },
    );
    Ok(Some((
        source_id.clone(),
        json!({
            "file":file,
            "startLine":start_line,
            "endLine":end_line,
            "byteStart":start,
            "byteEnd":end,
            "sourceId":source_id,
            "sourceDigest":text_digest,
            "evidenceDigest":binding,
            "sourceStatus":"SOURCE_RETAINED",
            "fullCompilationSourceDigest":blob.content_digest
        }),
    )))
}

fn add_kotlin_local_cfg(
    evidence: &mut ServiceEvidence,
    service: &Service,
    sources: &CompilationSource,
    known: &BTreeSet<String>,
    fact: &Value,
    binding: &str,
) -> Result<Option<Observation>, ClewError> {
    let graph: crate::thread_flow_cfg::LocalCfgPayload = serde_json::from_value(
        fact.get("graph")
            .cloned()
            .ok_or_else(|| invalid("Kotlin documentation CFG lacks its compiler graph"))?,
    )
    .map_err(|_| invalid("Kotlin documentation CFG graph violates the closed contract"))?;
    crate::thread_flow_cfg::validate(&graph)?;
    let owner = fact["ownerSymbolIdentity"]
        .as_str()
        .ok_or_else(|| invalid("Kotlin documentation CFG lacks its exact owner"))?;
    let graph_binding = fact["graphEvidenceBinding"]
        .as_str()
        .ok_or_else(|| invalid("Kotlin documentation CFG lacks its graph evidence binding"))?;
    let descriptor_binding = fact["descriptorEvidenceBinding"]
        .as_str()
        .ok_or_else(|| invalid("Kotlin documentation CFG lacks its descriptor evidence binding"))?;
    let raw_scope = fact["scope"]
        .as_str()
        .ok_or_else(|| invalid("Kotlin documentation CFG lacks its canonical compilation scope"))?;
    let scope = resolve_scope_key(&json!({"compilation":raw_scope}), known)?;
    let (Some(owner_start), Some(owner_end)) =
        (fact["ownerStart"].as_u64(), fact["ownerEnd"].as_u64())
    else {
        return Err(invalid(
            "Kotlin documentation CFG owner byte range is missing",
        ));
    };
    crate::semantic_validation::validate_kotlin_full_symbol_identity(owner)?;
    if fact["schema"] != LOCAL_CFG_EVIDENCE_SCHEMA
        || fact["kind"] != "LOCAL_CFG"
        || fact.get("graphEvidenceBinding").and_then(Value::as_str) != Some(binding)
        || graph_binding != binding
        || !super::local_cfg::canonical_evidence_binding(graph_binding)
        || !super::local_cfg::canonical_evidence_binding(descriptor_binding)
        || graph.owner_symbol_identity != owner
        || fact["file"] != graph.file
        || owner_start >= owner_end
    {
        return Err(invalid(
            "Kotlin documentation CFG identity or evidence bindings disagree",
        ));
    }
    store::relative(&graph.file)?;
    let Some(blob) = sources.blob(&scope, &graph.file) else {
        let boundary = LocalCfgBoundaryEvidence {
            schema: super::local_cfg::LOCAL_CFG_BOUNDARY_EVIDENCE_SCHEMA.into(),
            kind: "LOCAL_CFG_BOUNDARY".into(),
            scope: scope.clone(),
            owner_symbol_identity: Some(owner.into()),
            file: Some(graph.file.clone()),
            compiler_graph_name: Some(graph.compiler_graph_name.clone()),
            code: "KOTLIN_LOCAL_CFG_SOURCE_UNAVAILABLE".into(),
            provider: "CODECLEW_LOCAL_CFG_NORMALIZER".into(),
            evidence_binding: graph_binding.into(),
            descriptor_evidence_binding: Some(descriptor_binding.into()),
            raw_row_hash: None,
        };
        boundary.validate(binding)?;
        let normalized = serde_json::to_value(boundary)
            .map_err(|_| invalid("Kotlin local CFG source boundary cannot be retained"))?;
        let id = dependency_id(
            &service.id,
            "local-cfg-boundary",
            &scoped_identity(&scope, &digest(&normalized)?),
        )?;
        return Ok(Some(Observation {
            id,
            kind: "LOCAL_CFG_BOUNDARY".into(),
            service: service.id.clone(),
            symbol: owner.into(),
            digest: digest(&normalized)?,
            normalized,
            source_ids: Vec::new(),
        }));
    };
    let start = usize::try_from(owner_start)
        .map_err(|_| invalid("Kotlin CFG owner start exceeds addressable bytes"))?;
    let end = usize::try_from(owner_end)
        .map_err(|_| invalid("Kotlin CFG owner end exceeds addressable bytes"))?;
    if start >= end
        || end > blob.text.len()
        || !blob.text.is_char_boundary(start)
        || !blob.text.is_char_boundary(end)
    {
        return Err(invalid(
            "Kotlin CFG owner span is outside retained UTF-8 source boundaries",
        ));
    }
    let mut node_citations = Vec::<LocalCfgNodeCitation>::new();
    for node in &graph.nodes {
        let Some(range) = &node.source else {
            continue;
        };
        let node_start = usize::try_from(range.start)
            .map_err(|_| invalid("Kotlin CFG node start exceeds addressable bytes"))?;
        let node_end = usize::try_from(range.end)
            .map_err(|_| invalid("Kotlin CFG node end exceeds addressable bytes"))?;
        if node_start < start
            || node_end > end
            || node_start >= node_end
            || !blob.text.is_char_boundary(node_start)
            || !blob.text.is_char_boundary(node_end)
        {
            return Err(invalid(
                "Kotlin CFG node source range escapes its exact owner or UTF-8 boundary",
            ));
        }
        let exact = &blob.text[node_start..node_end];
        node_citations.push(LocalCfgNodeCitation {
            node_id: node.node_id,
            byte_start: (node_start - start) as u64,
            byte_end: (node_end - start) as u64,
            text_digest: canonical::hash_bytes(exact.as_bytes()),
        });
    }
    let text = &blob.text[start..end];
    let start_line = 1 + blob.text.as_bytes()[..start]
        .iter()
        .filter(|byte| **byte == b'\n')
        .count() as u64;
    let end_line = 1 + blob.text.as_bytes()[..end - 1]
        .iter()
        .filter(|byte| **byte == b'\n')
        .count() as u64;
    let identity = scoped_identity(
        &scope,
        &format!(
            "kotlin-local-cfg-owner:{owner}:{}:{owner_start}:{owner_end}",
            graph.file
        ),
    );
    let cfg_file = graph.file.clone();
    let graph_id = graph.graph_id.clone();
    let source = source_id(&service.id, &identity)?;
    let transformed = sources.transformed.get(&scope).copied().unwrap_or(false);
    let (authority, url): (String, Option<String>) = if transformed {
        (
            crate::generation_service::TRANSFORMED_SOURCE_AUTHORITY.into(),
            None,
        )
    } else {
        (
            "EXACT_SNAPSHOT_TEXT".into(),
            source_link(
                service,
                &evidence.revision,
                &graph.file,
                start_line,
                end_line,
            ),
        )
    };
    let source_digest = canonical::hash_bytes(text.as_bytes());
    evidence.sources.insert(
        source.clone(),
        Source {
            id: source.clone(),
            service: service.id.clone(),
            revision: evidence.revision.clone(),
            file: graph.file.clone(),
            start_line,
            end_line,
            text: text.into(),
            text_digest: source_digest.clone(),
            evidence_digest: graph_binding.into(),
            authority: authority.clone(),
            occurrence: None,
            url,
        },
    );
    let normalized = LocalCfgEvidence {
        schema: LOCAL_CFG_EVIDENCE_SCHEMA.into(),
        scope: scope.clone(),
        owner_symbol_identity: owner.into(),
        file: graph.file.clone(),
        graph,
        graph_evidence_binding: graph_binding.into(),
        descriptor_evidence_binding: descriptor_binding.into(),
        node_citations,
        source_site: LocalCfgSourceSite {
            source_id: source.clone(),
            source_digest,
            source_evidence_digest: graph_binding.into(),
            source_status: "SOURCE_RETAINED".into(),
            authority,
            file: cfg_file,
            start_line,
            end_line,
            owner_byte_start: owner_start,
            owner_byte_end: owner_end,
            source_content_digest: blob.content_digest.clone(),
            full_compilation_source_digest: blob.content_digest.clone(),
            span_digest: canonical::hash_bytes(text.as_bytes()),
        },
    };
    normalized.validate_source(
        &evidence.sources[&source],
        &evidence.service,
        &evidence.revision,
    )?;
    let normalized = serde_json::to_value(normalized)
        .map_err(|_| invalid("Kotlin compiler CFG cannot be retained"))?;
    let id = dependency_id(
        &service.id,
        "local-cfg",
        &scoped_identity(&scope, &graph_id),
    )?;
    Ok(Some(Observation {
        id,
        kind: "LOCAL_CFG".into(),
        service: service.id.clone(),
        symbol: owner.into(),
        digest: digest(&normalized)?,
        normalized,
        source_ids: vec![source],
    }))
}

/// Javac LineMap recognizes CR, LF and CRLF. This is intentionally separate
/// from the historical SourceBlob/snippet contract used by older fact kinds.
fn javac_line_ranges(text: &str) -> Vec<Range<usize>> {
    let bytes = text.as_bytes();
    let mut ranges = Vec::new();
    let mut start = 0;
    let mut index = 0;
    while index < bytes.len() {
        if matches!(bytes[index], b'\r' | b'\n') {
            ranges.push(start..index);
            if bytes[index] == b'\r' && bytes.get(index + 1) == Some(&b'\n') {
                index += 1;
            }
            index += 1;
            start = index;
        } else {
            index += 1;
        }
    }
    if start < bytes.len() {
        ranges.push(start..bytes.len());
    }
    ranges
}

type VariableSourceLines = BTreeMap<(String, String), Vec<Range<usize>>>;

#[allow(clippy::too_many_arguments)]
fn add_variable_source(
    evidence: &mut ServiceEvidence,
    service: &Service,
    sources: &CompilationSource,
    scope: &str,
    fact: &Value,
    binding: &str,
    identity: &str,
    lines: &[Range<usize>],
) -> Result<(String, usize, usize), ClewError> {
    let file = fact["file"].as_str().unwrap();
    let start = fact["startLine"].as_u64().unwrap();
    let end = fact["endLine"].as_u64().unwrap();
    if start == 0 || end < start || end > lines.len() as u64 {
        return Err(invalid("compiler variable source line range is invalid"));
    }
    let blob = sources.blob(scope, file).unwrap();
    // Preserve every original intermediate terminator, including bare CR;
    // omit only the terminator following the final selected context line.
    let source_start = lines[start as usize - 1].start;
    let source_end = lines[end as usize - 1].end;
    let exact = &blob.text[source_start..source_end];
    let id = source_id(&service.id, identity)?;
    let transformed = sources.transformed.get(scope).copied().unwrap_or(false);
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
            text: exact.into(),
            text_digest: canonical::hash_bytes(exact.as_bytes()),
            evidence_digest: binding.into(),
            authority,
            occurrence: None,
            url,
        },
    );
    Ok((id, source_start, source_end))
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

/// Fail closed before retaining compiler variable facts. Source/target membership is
/// checked within the exact compilation, never borrowed from another scope.
fn validate_variable_facts(
    facts: &[(Value, String)],
    sources: &CompilationSource,
    known: &BTreeSet<String>,
) -> Result<VariableSourceLines, ClewError> {
    use crate::java_adapter_v2::{JavaCompilerFact, validate_fact};
    if !facts.iter().any(|(f, _)| {
        matches!(
            f["kind"].as_str(),
            Some("VARIABLE_DECLARATION" | "VARIABLE_ACCESS")
        )
    }) {
        return Ok(BTreeMap::new());
    }
    let mut declarations: BTreeMap<(String, String), Vec<&Value>> = BTreeMap::new();
    let mut memberships: BTreeMap<(String, String), Vec<&Value>> = BTreeMap::new();
    for (fact, _) in facts {
        let kind = fact["kind"].as_str().unwrap_or_default();
        if matches!(kind, "DECLARATION" | "VARIABLE_DECLARATION") {
            let scope = resolve_scope_key(&fact["scope"], known)?;
            let identity = if kind == "DECLARATION" {
                &fact["symbolIdentity"]
            } else {
                &fact["variableIdentity"]
            };
            declarations
                .entry((scope, identity.as_str().unwrap_or_default().into()))
                .or_default()
                .push(fact);
        } else if kind == "SOURCE_FILE" {
            let scope = resolve_scope_key(&fact["scope"], known)?;
            memberships
                .entry((scope, fact["file"].as_str().unwrap_or_default().into()))
                .or_default()
                .push(fact);
        }
    }
    let mut wanted: BTreeMap<(String, String), BTreeSet<usize>> = BTreeMap::new();
    for (fact, _) in facts {
        if !matches!(
            fact["kind"].as_str(),
            Some("VARIABLE_DECLARATION" | "VARIABLE_ACCESS")
        ) {
            continue;
        }
        let scope = resolve_scope_key(&fact["scope"], known)?;
        let file = fact["file"]
            .as_str()
            .ok_or_else(|| invalid("compiler variable file is missing"))?;
        let offsets = wanted.entry((scope, file.into())).or_default();
        for key in ["byteStart", "byteEnd"] {
            if let Some(offset) = fact[key].as_u64().and_then(|v| usize::try_from(v).ok()) {
                offsets.insert(offset);
            }
        }
    }
    let mut coordinates: BTreeMap<(String, String), BTreeMap<usize, u64>> = BTreeMap::new();
    for ((scope, file), wanted) in wanted {
        let blob = sources
            .blob(&scope, &file)
            .ok_or_else(|| invalid("compiler variable source is unavailable in its compilation"))?;
        let mut utf16 = 0u64;
        let mut offsets = BTreeMap::new();
        for (byte, ch) in blob.text.char_indices() {
            if wanted.contains(&byte) {
                offsets.insert(byte, utf16);
            }
            utf16 += ch.len_utf16() as u64;
        }
        if wanted.contains(&blob.text.len()) {
            offsets.insert(blob.text.len(), utf16);
        }
        coordinates.insert((scope, file), offsets);
    }
    let variable_lines: VariableSourceLines = coordinates
        .keys()
        .map(|(scope, file)| {
            (
                (scope.clone(), file.clone()),
                javac_line_ranges(&sources.blob(scope, file).unwrap().text),
            )
        })
        .collect();
    let mut occurrences = BTreeSet::new();
    for (fact, binding) in facts {
        if !matches!(
            fact["kind"].as_str(),
            Some("VARIABLE_DECLARATION" | "VARIABLE_ACCESS")
        ) {
            continue;
        }
        let scope = resolve_scope_key(&fact["scope"], known)?;
        let occurrence = (
            scope.clone(),
            fact["kind"].to_string(),
            fact["enclosingCallable"].to_string(),
            fact["occurrencePath"].to_string(),
        );
        if !occurrences.insert(occurrence) {
            return Err(invalid("compiler variable occurrence is duplicated"));
        }
        if scope.is_empty() {
            return Err(invalid(
                "compiler variable facts require a registered scope",
            ));
        }
        let mut raw = fact.clone();
        raw.as_object_mut().unwrap().remove("scope");
        let typed: JavaCompilerFact = serde_json::from_value(raw.clone())
            .map_err(|_| invalid("compiler variable fact is not closed typed evidence"))?;
        validate_fact(&typed)?;
        // Compiler payload references are CAS identities, including the schema
        // domain. An unsalted semantic JSON hash is not their provenance.
        let payload = crate::cas::CasObject::for_bytes(
            crate::java_adapter_v2::JAVA_FACT_SCHEMA,
            &bytes(&raw)?,
        )?;
        if payload.digest != *binding {
            return Err(invalid("compiler variable payload binding differs"));
        }
        let file = fact["file"].as_str().unwrap();
        let blob = sources
            .blob(&scope, file)
            .ok_or_else(|| invalid("compiler variable source is unavailable in its compilation"))?;
        let full_digest = &blob.content_digest;
        let pins = memberships
            .get(&(scope.clone(), file.into()))
            .ok_or_else(|| invalid("compiler variable source membership is missing"))?;
        if pins.len() != 1 || pins[0]["sourceContentDigest"].as_str() != Some(full_digest.as_str())
        {
            return Err(invalid(
                "compiler variable source differs from its immutable membership",
            ));
        }
        let start = usize::try_from(fact["byteStart"].as_u64().unwrap())
            .map_err(|_| invalid("compiler variable byte start is invalid"))?;
        let end = usize::try_from(fact["byteEnd"].as_u64().unwrap())
            .map_err(|_| invalid("compiler variable byte end is invalid"))?;
        let _exact = blob
            .text
            .get(start..end)
            .ok_or_else(|| invalid("compiler variable byte span is not exact UTF-8"))?;
        let offsets = &coordinates[&(scope.clone(), file.into())];
        let utf16_start = offsets[&start];
        let utf16_end = offsets[&end];
        let lines = &variable_lines[&(scope.clone(), file.into())];
        let first_line = lines.partition_point(|range| range.start <= start) as u64;
        let last_line = lines.partition_point(|range| range.start < end) as u64;
        if fact["start"] != utf16_start
            || fact["end"] != utf16_end
            || fact["startLine"] != first_line
            || fact["endLine"] != last_line
        {
            return Err(invalid(
                "compiler variable source coordinates disagree with immutable bytes",
            ));
        }
        let callable = fact["enclosingCallable"].as_str().unwrap();
        let owners = declarations
            .get(&(scope.clone(), callable.into()))
            .ok_or_else(|| invalid("compiler variable callable is not retained in its scope"))?;
        if owners.len() != 1
            || !matches!(
                owners[0]["declarationKind"].as_str(),
                Some("METHOD" | "CONSTRUCTOR")
            )
            || owners[0]["file"] != file
            || owners[0]["byteStart"]
                .as_u64()
                .is_none_or(|value| value > start as u64)
            || owners[0]["byteEnd"]
                .as_u64()
                .is_none_or(|value| value < end as u64)
        {
            return Err(invalid(
                "compiler variable callable source binding is invalid",
            ));
        }
        if fact["variableKind"] != "FIELD" && fact["variableOwnerIdentity"] != callable {
            return Err(invalid(
                "compiler variable owner differs from immediate callable",
            ));
        }
        if fact["kind"] == "VARIABLE_ACCESS" {
            let identity = fact["variableIdentity"].as_str().unwrap();
            let target = declarations.get(&(scope.clone(), identity.into()));
            if fact["declarationStatus"] == "DECLARATION_SOURCE_UNAVAILABLE" {
                if fact["variableKind"] != "FIELD" || target.is_some() {
                    return Err(invalid(
                        "unavailable variable declaration conflicts with retained scope",
                    ));
                }
            } else {
                let target = target.filter(|rows| rows.len() == 1).ok_or_else(|| {
                    invalid("variable target is missing or ambiguous in its scope")
                })?[0];
                let field = fact["variableKind"] == "FIELD";
                if target["name"] != fact["name"]
                    || target["jvmDescriptor"] != fact["jvmDescriptor"]
                    || (field
                        && (target["declarationKind"] != "FIELD"
                            || target["ownerIdentity"] != fact["variableOwnerIdentity"]))
                    || (!field
                        && (target["variableKind"] != fact["variableKind"]
                            || target["enclosingCallable"] != callable))
                {
                    return Err(invalid(
                        "compiler variable resolved target metadata differs",
                    ));
                }
                let target_file = target["file"]
                    .as_str()
                    .ok_or_else(|| invalid("variable target source path is missing"))?;
                let target_blob = sources.blob(&scope, target_file).ok_or_else(|| {
                    invalid("variable target source is not retained in its scope")
                })?;
                let target_pins = memberships
                    .get(&(scope.clone(), target_file.into()))
                    .ok_or_else(|| invalid("variable target source membership is missing"))?;
                if target_pins.len() != 1
                    || target_pins[0]["sourceContentDigest"].as_str()
                        != Some(target_blob.content_digest.as_str())
                {
                    return Err(invalid(
                        "variable target source differs from immutable membership",
                    ));
                }
                let (Some(target_start), Some(target_end)) =
                    (target["byteStart"].as_u64(), target["byteEnd"].as_u64())
                else {
                    return Err(invalid(
                        "variable target declaration lacks exact source span",
                    ));
                };
                if target_start >= target_end
                    || target_blob
                        .text
                        .get(target_start as usize..target_end as usize)
                        .is_none()
                {
                    return Err(invalid(
                        "variable target declaration source span is invalid",
                    ));
                }
            }
        }
    }
    Ok(variable_lines)
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
    let variable_lines = validate_variable_facts(&facts, sources, known)?;
    let annotation_registry = spring_entrypoints::annotation_registry(facts.iter().map(|(f, _)| f));
    // Track, per symbol, the distinct normalized payload digests observed
    // across admitted compilation scopes. A symbol that resolves to multiple
    // incompatible candidates becomes an explicit SCOPE_AMBIGUOUS boundary
    // rather than last-write-wins overwriting a single candidate.
    let mut symbol_digests: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for (fact, binding) in &facts {
        if fact["schema"] == "codeclew-kotlin-documentation-local-cfg-boundary/1.0" {
            let mut boundary: LocalCfgBoundaryEvidence = serde_json::from_value(fact.clone())
                .map_err(|_| invalid("Kotlin CFG boundary violates its typed contract"))?;
            boundary.validate(binding)?;
            let scope = resolve_scope_key(&json!({"compilation":boundary.scope}), known)?;
            boundary.scope = scope.clone();
            let owner = boundary.owner_symbol_identity.clone();
            let normalized = serde_json::to_value(boundary)
                .map_err(|_| invalid("Kotlin CFG boundary cannot be retained"))?;
            let id = dependency_id(
                &service.id,
                "local-cfg-boundary",
                &scoped_identity(&scope, &digest(&normalized)?),
            )?;
            evidence.observations.insert(
                id.clone(),
                Observation {
                    id,
                    kind: "LOCAL_CFG_BOUNDARY".into(),
                    service: service.id.clone(),
                    symbol: owner.unwrap_or_default(),
                    digest: digest(&normalized)?,
                    normalized,
                    source_ids: Vec::new(),
                },
            );
            continue;
        }
        if fact["kind"] == "BOUNDARY" {
            evidence.boundaries.push(
                fact["code"]
                    .as_str()
                    .unwrap_or("JVM_ANALYSIS_BOUNDARY")
                    .into(),
            );
            continue;
        }
        // An exact dependency declaration identifies the admitted binary
        // target. Its source/body availability is independent of the caller's
        // retained call-site source; never manufacture a Source from metadata.
        if fact["kind"] == "DEPENDENCY_TARGET" {
            let symbol = fact["symbolIdentity"]
                .as_str()
                .filter(|identity| !identity.is_empty())
                .ok_or_else(|| invalid("compiler dependency target lacks identity"))?;
            let scope = resolve_scope_key(&fact["scope"], known)?;
            let identity = scoped_identity(&scope, symbol);
            let id = dependency_id(&service.id, "dependency-target", &identity)?;
            let mut normalized = strip_coordinates(fact);
            normalized["scope"] = json!(scope);
            let mut source_ids = Vec::new();
            if fact["sourceStatus"] == "SOURCE_ATTACHED" && !fact["dependencySource"].is_object() {
                return Err(invalid(
                    "attached dependency source lacks its exact payload",
                ));
            }
            if let Some(attached) = fact["dependencySource"].as_object() {
                let text = attached
                    .get("text")
                    .and_then(Value::as_str)
                    .ok_or_else(|| invalid("dependency source attachment lacks exact text"))?;
                let file = attached
                    .get("sourceEntry")
                    .and_then(Value::as_str)
                    .ok_or_else(|| invalid("dependency source attachment lacks archive entry"))?;
                store::relative(file)?;
                let start_line = attached
                    .get("startLine")
                    .and_then(Value::as_u64)
                    .ok_or_else(|| invalid("dependency source attachment lacks start line"))?;
                let end_line = attached
                    .get("endLine")
                    .and_then(Value::as_u64)
                    .ok_or_else(|| invalid("dependency source attachment lacks end line"))?;
                let text_digest = canonical::hash_bytes(text.as_bytes());
                if fact["sourceStatus"] != "SOURCE_ATTACHED"
                    || attached.get("textDigest").and_then(Value::as_str)
                        != Some(text_digest.as_str())
                    || start_line == 0
                    || end_line < start_line
                    || end_line - start_line + 1 != text.lines().count() as u64
                {
                    return Err(invalid(
                        "dependency source attachment binding is inconsistent",
                    ));
                }
                let source_id = source_id(&service.id, &format!("dependency-target:{identity}"))?;
                evidence.sources.insert(
                    source_id.clone(),
                    Source {
                        id: source_id.clone(),
                        service: service.id.clone(),
                        revision: revision.into(),
                        file: file.into(),
                        start_line,
                        end_line,
                        text: text.into(),
                        text_digest,
                        evidence_digest: binding.clone(),
                        authority: "EXACT_DEPENDENCY_SOURCE_ARCHIVE".into(),
                        occurrence: None,
                        url: None,
                    },
                );
                // Source payloads are shared immutable records. The observation
                // retains archive/signature provenance, never a second text copy.
                let mut metadata = fact["dependencySource"].clone();
                metadata.as_object_mut().unwrap().remove("text");
                metadata["sourceId"] = json!(source_id);
                normalized["dependencySource"] = metadata;
                source_ids.push(source_id);
            }
            evidence.observations.insert(
                id.clone(),
                Observation {
                    id,
                    kind: "DEPENDENCY_TARGET".into(),
                    service: service.id.clone(),
                    symbol: symbol.into(),
                    digest: digest(&normalized)?,
                    normalized,
                    source_ids,
                },
            );
            continue;
        }
        if matches!(
            fact["kind"].as_str(),
            Some("VARIABLE_DECLARATION" | "VARIABLE_ACCESS")
        ) {
            let scope = resolve_scope_key(&fact["scope"], known)?;
            let declaration = fact["kind"] == "VARIABLE_DECLARATION";
            let callable = fact["enclosingCallable"].as_str().unwrap();
            let variable = fact["variableIdentity"].as_str().unwrap();
            let logical = if declaration {
                variable.to_string()
            } else {
                format!("{callable}/{}", fact["occurrencePath"].as_str().unwrap())
            };
            let identity = scoped_identity(&scope, &logical);
            let id = dependency_id(
                &service.id,
                if declaration {
                    "variable-declaration"
                } else {
                    "variable-access"
                },
                &identity,
            )?;
            let (source, source_start, source_end) = add_variable_source(
                &mut evidence,
                service,
                sources,
                &scope,
                fact,
                binding,
                &identity,
                &variable_lines[&(scope.clone(), fact["file"].as_str().unwrap().into())],
            )?;
            let blob = sources
                .blob(&scope, fact["file"].as_str().unwrap())
                .unwrap();
            let start = fact["byteStart"].as_u64().unwrap() as usize;
            let end = fact["byteEnd"].as_u64().unwrap() as usize;
            let mut normalized = strip_coordinates(fact);
            normalized["scope"] = json!(scope);
            normalized["callableObservationId"] = json!(dependency_id(
                &service.id,
                "symbol",
                &scoped_identity(&scope, callable)
            )?);
            if !declaration && fact["declarationStatus"] == "SOURCE_RETAINED" {
                normalized["declarationObservationId"] = json!(dependency_id(
                    &service.id,
                    if fact["variableKind"] == "FIELD" {
                        "symbol"
                    } else {
                        "variable-declaration"
                    },
                    &scoped_identity(&scope, variable)
                )?);
            }
            normalized["variableSite"] = json!({"file":fact["file"],"service":service.id,"revision":revision,
                "byteStart":start,"byteEnd":end,"sourceByteStart":source_start,"sourceByteEnd":source_end,
                "startLine":fact["startLine"],"endLine":fact["endLine"],
                "sourceContentDigest":blob.content_digest,
                "spanDigest":canonical::hash_bytes(blob.text[start..end].as_bytes()),
                "sourceId":source,"sourceDigest":evidence.sources[&source].text_digest,
                "evidenceDigest":binding,"sourceStatus":"SOURCE_RETAINED"});
            evidence.observations.insert(
                id.clone(),
                Observation {
                    id,
                    kind: fact["kind"].as_str().unwrap().into(),
                    service: service.id.clone(),
                    symbol: callable.into(),
                    digest: digest(&normalized)?,
                    normalized,
                    source_ids: vec![source],
                },
            );
            continue;
        }
        if fact["schema"] == "codeclew-kotlin-documentation-call/1.0" {
            let source_callable = fact["sourceCompilerCallableId"]
                .as_str()
                .ok_or_else(|| invalid("Kotlin CALLS relation lacks exact source callable id"))?;
            let source_descriptor = fact["sourceJvmDescriptor"].as_str().ok_or_else(|| {
                invalid("Kotlin CALLS relation lacks exact source JVM descriptor")
            })?;
            let target_callable = fact["targetCompilerCallableId"]
                .as_str()
                .ok_or_else(|| invalid("Kotlin CALLS relation lacks exact target callable id"))?;
            let target_descriptor = fact["targetJvmDescriptor"].as_str().ok_or_else(|| {
                invalid("Kotlin CALLS relation lacks exact target JVM descriptor")
            })?;
            let source_identity = format!("callable:{source_callable}#jvm:{source_descriptor}");
            let target_identity = format!("callable:{target_callable}#jvm:{target_descriptor}");
            if fact["kind"] != "RELATION"
                || fact["relationKind"] != "CALLS"
                || fact["resolution"] != "COMPILER_EXACT"
                || fact["compilerResolution"] != "PROVEN"
                || fact["provider"] != "K2_FIR"
                || fact["compilerSchema"] != "declaration-relation/0.1"
                || fact["sourceIdentity"] != source_identity
                || fact["targetIdentity"] != target_identity
                || fact["sourceProvenance"] != "COMPILER_UTF16_RANGE_TO_UTF8_BYTES"
                || fact["evidenceBinding"].as_str() != Some(binding)
            {
                return Err(invalid(
                    "Kotlin CALLS relation is not compiler-exact and payload-bound",
                ));
            }
            crate::semantic_validation::validate_kotlin_full_symbol_identity(&source_identity)?;
            crate::semantic_validation::validate_kotlin_full_symbol_identity(&target_identity)?;
            let scope = resolve_scope_key(&fact["scope"], known)?;
            let identity = scoped_identity(&scope, &format!("kotlin-call-site:{}", digest(fact)?));
            let Some((source_id, call_site)) = add_kotlin_call_site_source(
                &mut evidence,
                service,
                sources,
                &scope,
                fact,
                binding,
                &identity,
            )?
            else {
                if !evidence
                    .boundaries
                    .iter()
                    .any(|boundary| boundary == "KOTLIN_CALL_SOURCE_UNAVAILABLE")
                {
                    evidence
                        .boundaries
                        .push("KOTLIN_CALL_SOURCE_UNAVAILABLE".into());
                }
                continue;
            };
            let mut normalized = strip_coordinates(fact);
            normalized["scope"] = json!(scope);
            normalized["callSite"] = call_site;
            let id = dependency_id(&service.id, "call-relation", &identity)?;
            evidence.observations.insert(
                id.clone(),
                Observation {
                    id,
                    kind: "CALL_RELATION".into(),
                    service: service.id.clone(),
                    symbol: source_identity,
                    digest: digest(&normalized)?,
                    normalized,
                    source_ids: vec![source_id],
                },
            );
            continue;
        }
        if fact["schema"] == "codeclew-kotlin-documentation-local-cfg/1.0" {
            if let Some(observation) =
                add_kotlin_local_cfg(&mut evidence, service, sources, known, fact, binding)?
            {
                evidence
                    .observations
                    .insert(observation.id.clone(), observation);
            }
            continue;
        }
        // Compiler relations have independent authority and source
        // coordinates. Retain them instead of folding them into FLOW, whose
        // source-order events have different limits and control-flow
        // boundaries. REFERENCES are compiler-confirmed method references;
        // they do not establish invocation or execution order.
        if fact["kind"] == "RELATION"
            && matches!(
                fact["relationKind"].as_str(),
                Some("CALLS" | "CONSTRUCTS" | "REFERENCES" | "TYPE_USES")
            )
        {
            let scope = resolve_scope_key(&fact["scope"], known)?;
            let is_type_use = fact["relationKind"] == "TYPE_USES";
            let relation_prefix = if is_type_use {
                "type-site"
            } else if fact["relationKind"] == "REFERENCES" {
                "reference-site"
            } else {
                "call-site"
            };
            let identity = scoped_identity(&scope, &format!("{relation_prefix}:{}", digest(fact)?));
            let id = dependency_id(&service.id, "call-relation", &identity)?;
            let source = add_source(
                &mut evidence,
                service,
                sources,
                &scope,
                fact,
                binding,
                &identity,
            )?;
            let mut normalized = strip_coordinates(fact);
            normalized["scope"] = json!(scope);
            let mut call_site = json!({
                "file":fact["file"],
                "startLine":fact["startLine"],
                "endLine":fact["endLine"],
                "byteStart":fact["byteStart"],
                "byteEnd":fact["byteEnd"],
                "sourceStatus":"SOURCE_UNAVAILABLE"
            });
            if fact["sourceIdentity"].as_str().is_none_or(str::is_empty) {
                call_site["ownerStatus"] = json!("SOURCE_OWNER_UNAVAILABLE");
                let boundary = if is_type_use {
                    "TYPE_RELATION_OWNER_UNAVAILABLE"
                } else if fact["relationKind"] == "REFERENCES" {
                    "REFERENCE_RELATION_OWNER_UNAVAILABLE"
                } else {
                    "CALL_RELATION_OWNER_UNAVAILABLE"
                };
                if !evidence
                    .boundaries
                    .iter()
                    .any(|existing| existing == boundary)
                {
                    evidence.boundaries.push(boundary.into());
                }
            } else {
                call_site["ownerStatus"] = json!("SOURCE_OWNER_RETAINED");
            }
            let source_ids: Vec<_> = source.into_iter().collect();
            if let Some(source_id) = source_ids.first()
                && let Some(source) = evidence.sources.get(source_id)
            {
                call_site["sourceId"] = json!(source_id);
                call_site["sourceDigest"] = json!(source.text_digest);
                call_site["evidenceDigest"] = json!(source.evidence_digest);
                call_site["sourceStatus"] = json!("SOURCE_RETAINED");
            }
            normalized[if is_type_use { "typeSite" } else { "callSite" }] = call_site;
            let symbol = fact["sourceIdentity"].as_str().unwrap_or_default();
            evidence.observations.insert(
                id.clone(),
                Observation {
                    id,
                    kind: if is_type_use {
                        "TYPE_RELATION"
                    } else {
                        "CALL_RELATION"
                    }
                    .into(),
                    service: service.id.clone(),
                    symbol: symbol.into(),
                    digest: digest(&normalized)?,
                    normalized,
                    source_ids,
                },
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
        if service.language == "csharp" {
            let Some(metadata) = crate::aspnetcore_entrypoints::metadata_for_fact(fact)? else {
                continue;
            };
            for (ordinal, entry) in metadata.entries.iter().enumerate() {
                let identity = format!("{symbol}/{ordinal}");
                let eid = source_id(
                    &service.id,
                    &scoped_identity(&scope, &format!("entrypoint/{identity}")),
                )?;
                let trigger = crate::aspnetcore_entrypoints::describe_trigger(entry);
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
                        symbol: symbol.into(),
                        digest: digest(&normalized)?,
                        normalized,
                        source_ids: source_ids.clone(),
                    },
                );
                evidence.entrypoints.push(Entrypoint {
                    id: eid,
                    service: service.id.clone(),
                    symbol: symbol.into(),
                    kind: entry.kind.clone(),
                    trigger,
                    source_ids: source_ids.clone(),
                    dependency_ids: vec![id.clone(), route_id],
                    boundaries: metadata.boundaries.clone(),
                });
            }
            continue;
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

/// Validate present column bounds without trusting normalized JSON. Old variable
/// sites without either bound remain readable, but cannot supply an exact column.
fn verify_variable_site(e: &ServiceEvidence, observation: &Observation) -> Result<(), ClewError> {
    let site = &observation.normalized["variableSite"];
    if site.get("sourceByteStart").is_none() && site.get("sourceByteEnd").is_none() {
        return Ok(());
    }
    let number = |key: &str| {
        site[key]
            .as_u64()
            .ok_or_else(|| invalid("portable variable site byte bound is invalid"))
    };
    let source_start = number("sourceByteStart")?;
    let source_end = number("sourceByteEnd")?;
    let start = number("byteStart")?;
    let end = number("byteEnd")?;
    let source_id = site["sourceId"]
        .as_str()
        .ok_or_else(|| invalid("portable variable site source ID is missing"))?;
    let source = e
        .sources
        .get(source_id)
        .ok_or_else(|| invalid("portable variable site source is unavailable"))?;
    if observation.source_ids.len() != 1
        || observation.source_ids[0] != source_id
        || source.id != source_id
        || observation.service != e.service
        || source.service != e.service
        || site["service"] != e.service
        || site["revision"] != e.revision
        || source.revision != e.revision
        || site["file"] != source.file
        || site["sourceDigest"] != source.text_digest
        || site["evidenceDigest"] != source.evidence_digest
        || source.text_digest != canonical::hash_bytes(source.text.as_bytes())
        || site["sourceStatus"] != "SOURCE_RETAINED"
        || site["startLine"] != source.start_line
        || site["endLine"] != source.end_line
        || source_start > start
        || start >= end
        || end > source_end
        || source_end.checked_sub(source_start) != Some(source.text.len() as u64)
    {
        return Err(invalid(
            "portable variable source span or pins are inconsistent",
        ));
    }
    let relative_start = usize::try_from(start - source_start)
        .map_err(|_| invalid("portable variable local byte start is invalid"))?;
    let relative_end = usize::try_from(end - source_start)
        .map_err(|_| invalid("portable variable local byte end is invalid"))?;
    let exact = source
        .text
        .get(relative_start..relative_end)
        .ok_or_else(|| invalid("portable variable local span is not UTF-8"))?;
    if site["spanDigest"] != canonical::hash_bytes(exact.as_bytes()) {
        return Err(invalid("portable variable local span digest differs"));
    }
    Ok(())
}

pub fn verify_evidence(e: &ServiceEvidence) -> Result<(), ClewError> {
    if e.schema != "codeclew-documentation-service-evidence/1.0"
        || e.revision.len() != 40
        || !e.revision.bytes().all(|b| b.is_ascii_hexdigit())
        || ![EXTRACTOR, SOURCE_EXTRACTOR].contains(&e.extractor.as_str())
    {
        return Err(invalid("portable evidence version or revision is invalid"));
    }
    let variable_sources: BTreeSet<_> = e
        .observations
        .values()
        .filter(|o| matches!(o.kind.as_str(), "VARIABLE_ACCESS" | "VARIABLE_DECLARATION"))
        .flat_map(|o| o.source_ids.iter())
        .collect();
    for s in e.sources.values() {
        let line_count = if variable_sources.contains(&s.id) {
            javac_line_ranges(&s.text).len()
        } else {
            s.text.lines().count()
        };
        if s.service != e.service
            || s.revision != e.revision
            || s.start_line == 0
            || s.end_line < s.start_line
            || s.end_line - s.start_line + 1 != line_count as u64
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
        if matches!(o.kind.as_str(), "VARIABLE_ACCESS" | "VARIABLE_DECLARATION") {
            verify_variable_site(e, o)?;
        }
        if o.kind == "LOCAL_CFG" {
            let source = match o.source_ids.as_slice() {
                [source_id] => e
                    .sources
                    .get(source_id)
                    .filter(|source| source.service == e.service && source.revision == e.revision)
                    .ok_or_else(|| invalid("portable Kotlin CFG source is unavailable"))?,
                _ => return Err(invalid("portable Kotlin CFG source binding is ambiguous")),
            };
            if !super::local_cfg::LocalCfgEvidence::source_bound(
                &o.normalized,
                source,
                &e.service,
                &e.revision,
            ) {
                return Err(invalid(
                    "portable Kotlin CFG source binding is inconsistent",
                ));
            }
        }
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

    fn java_fact_binding(value: &impl serde::Serialize) -> String {
        crate::cas::CasObject::for_bytes(
            crate::java_adapter_v2::JAVA_FACT_SCHEMA,
            &canonical::bytes(value).unwrap(),
        )
        .unwrap()
        .digest
    }
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

    fn call_relation_fact(
        scope: &Value,
        source: &str,
        target: Value,
        file: &str,
        line: u64,
        resolution: &str,
    ) -> (Value, String) {
        (
            json!({
                "kind":"RELATION","relationKind":"CALLS",
                "sourceIdentity":source,"targetIdentity":target,
                "resolution":resolution,"file":file,
                "startLine":line,"endLine":line,
                "byteStart":100,"byteEnd":110,"scope":scope,
            }),
            "call-binding-digest".into(),
        )
    }

    fn kotlin_call_relation_fact(
        scope: &Value,
        source: &str,
        target: &str,
        file: &str,
        start: u64,
        end: u64,
        binding: &str,
    ) -> (Value, String) {
        let (target_callable, target_descriptor) = target
            .strip_prefix("callable:")
            .unwrap()
            .split_once("#jvm:")
            .unwrap();
        let (source_callable, source_descriptor) = source
            .strip_prefix("callable:")
            .unwrap()
            .split_once("#jvm:")
            .unwrap();
        (
            json!({
                "schema":"codeclew-kotlin-documentation-call/1.0",
                "kind":"RELATION","relationKind":"CALLS",
                "sourceIdentity":source,"targetIdentity":target,
                "resolution":"COMPILER_EXACT","provider":"K2_FIR",
                "compilerResolution":"PROVEN","compilerSchema":"declaration-relation/0.1",
                "sourceCompilerCallableId":source_callable,"sourceJvmDescriptor":source_descriptor,
                "targetCompilerCallableId":target_callable,"targetJvmDescriptor":target_descriptor,
                "file":file,"byteStart":start,"byteEnd":end,
                "scope":scope,"sourceProvenance":"COMPILER_UTF16_RANGE_TO_UTF8_BYTES",
                "evidenceBinding":binding
            }),
            binding.into(),
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
    fn scoped_call_relation_keeps_try_call_source_and_exception_flow_boundary() {
        let service = projection_service();
        let file = "src/main/java/example/Service.java";
        let text = "package example;\nclass Service {\n  void endpoint() { try { helper(); } catch (RuntimeException ex) {} }\n  void helper() {}\n}\n";
        let table = BTreeMap::from([(file.into(), text.into())]);
        let sources = compile_sources(vec![(":a/main", table, true)], BTreeMap::new());
        let scope = json!({"compilation":":a/main"});
        let endpoint = "method:example.Service#endpoint()V";
        let helper = "method:example.Service#helper()V";
        let facts = vec![
            (
                json!({
                    "kind":"DECLARATION","symbolIdentity":endpoint,
                    "ownerIdentity":"class:example.Service","name":"endpoint",
                    "file":file,"startLine":3,"endLine":3,"scope":scope,
                    "documentation":{
                        "events":[{"kind":"BOUNDARY","file":file,"startLine":3,"endLine":3}],
                        "boundaries":["EXCEPTION_FLOW_REQUIRES_SOURCE_REVIEW"]
                    }
                }),
                "declaration-binding".into(),
            ),
            call_relation_fact(&scope, endpoint, json!(helper), file, 3, "COMPILER_EXACT"),
        ];

        let evidence = project_scoped_ok(&service, facts, &sources, &[":a/main"]);
        let relation = evidence
            .observations
            .values()
            .find(|observation| observation.kind == "CALL_RELATION")
            .expect("javac call relation is retained independently of FLOW");
        assert_eq!(relation.service, service.id);
        assert_eq!(relation.symbol, endpoint);
        assert_eq!(relation.normalized["relationKind"], "CALLS");
        assert_eq!(relation.normalized["sourceIdentity"], endpoint);
        assert_eq!(relation.normalized["targetIdentity"], helper);
        assert_eq!(relation.normalized["resolution"], "COMPILER_EXACT");
        assert_eq!(relation.normalized["scope"], ":a/main");
        assert_eq!(relation.source_ids.len(), 1);
        let source = &evidence.sources[&relation.source_ids[0]];
        assert!(source.text.contains("helper();"));
        assert_eq!(
            relation.normalized["callSite"]["sourceDigest"],
            source.text_digest
        );
        assert_eq!(
            relation.normalized["callSite"]["evidenceDigest"],
            source.evidence_digest
        );
        assert_eq!(
            relation.normalized["callSite"]["sourceStatus"],
            "SOURCE_RETAINED"
        );
        assert!(
            evidence
                .boundaries
                .iter()
                .any(|boundary| { boundary == "EXCEPTION_FLOW_REQUIRES_SOURCE_REVIEW" })
        );
        assert!(evidence.observations.values().any(|observation| {
            observation.kind == "FLOW" && observation.normalized["kind"] == "BOUNDARY"
        }));
    }

    #[test]
    fn kotlin_call_relation_retains_exact_utf8_occurrences_and_payload_bindings() {
        let service = projection_service();
        let file = "src/main/kotlin/example/Service.kt";
        let text = "fun endpoint() { val prefix = \"π🙂\"; api.pick(\"x\"); api.pick(\"x\") }\n";
        let scope = json!({"compilation":":kotlin/main"});
        let owner = "callable:example/Service.endpoint#jvm:()V";
        let target = "callable:example/Api.pick#jvm:(Ljava/lang/String;)Ljava/lang/String;";
        let call_text = "api.pick(\"x\")";
        let occurrences = text.match_indices(call_text).collect::<Vec<_>>();
        assert_eq!(
            occurrences.len(),
            2,
            "fixture must have two identical calls"
        );
        let (first_start, first_call) = occurrences[0];
        let (second_start, second_call) = occurrences[1];
        let first_end = first_start + first_call.len();
        let second_end = second_start + second_call.len();
        let facts = vec![
            kotlin_call_relation_fact(
                &scope,
                owner,
                target,
                file,
                first_start as u64,
                first_end as u64,
                "first-call-binding",
            ),
            kotlin_call_relation_fact(
                &scope,
                owner,
                target,
                file,
                second_start as u64,
                second_end as u64,
                "second-call-binding",
            ),
        ];
        let evidence = project_scoped_ok(
            &service,
            facts,
            &compile_sources(
                vec![(
                    ":kotlin/main",
                    BTreeMap::from([(file.into(), text.into())]),
                    true,
                )],
                BTreeMap::new(),
            ),
            &[":kotlin/main"],
        );
        let calls = evidence
            .observations
            .values()
            .filter(|observation| observation.kind == "CALL_RELATION")
            .collect::<Vec<_>>();
        assert_eq!(calls.len(), 2);
        let mut spans = BTreeSet::new();
        for call in &calls {
            let site = &call.normalized["callSite"];
            let start = site["byteStart"].as_u64().unwrap() as usize;
            let end = site["byteEnd"].as_u64().unwrap() as usize;
            assert!(
                spans.insert((start, end)),
                "same-line call ranges stay distinct"
            );
            let source = &evidence.sources[&call.source_ids[0]];
            assert_eq!(source.text, text[start..end]);
            assert_eq!(source.file, file);
            assert_eq!(source.occurrence, None);
            assert_eq!(site["sourceId"], source.id);
            assert_eq!(site["sourceDigest"], source.text_digest);
            assert_eq!(site["evidenceDigest"], source.evidence_digest);
            assert_eq!(site["sourceStatus"], "SOURCE_RETAINED");
            assert_eq!(
                site["fullCompilationSourceDigest"],
                crate::canonical::hash_bytes(text.as_bytes())
            );
            assert_eq!(site["startLine"], 1);
            assert_eq!(site["endLine"], 1);
            assert!(site.get("sourceOccurrence").is_none());
        }
        assert!(
            !evidence
                .observations
                .values()
                .any(|observation| observation.kind == "FLOW")
        );
        let evidence_bindings = calls
            .iter()
            .map(|call| {
                evidence.sources[&call.source_ids[0]]
                    .evidence_digest
                    .as_str()
            })
            .collect::<BTreeSet<_>>();
        assert_eq!(
            evidence_bindings,
            BTreeSet::from(["first-call-binding", "second-call-binding"])
        );
    }

    #[test]
    fn kotlin_call_relation_without_scoped_source_is_boundary_only_and_bad_ranges_fail_closed() {
        let service = projection_service();
        let file = "src/main/kotlin/example/Missing.kt";
        let text = "fun endpoint() { api.pick() }";
        let scope = json!({"compilation":":kotlin/main"});
        let owner = "callable:example/Service.endpoint#jvm:()V";
        let target = "callable:example/Api.pick#jvm:()V";
        let relation = kotlin_call_relation_fact(
            &scope,
            owner,
            target,
            file,
            18,
            29,
            "missing-source-binding",
        );
        let evidence = project_scoped_ok(
            &service,
            vec![relation],
            &compile_sources(
                vec![(":kotlin/main", BTreeMap::new(), false)],
                BTreeMap::from([(file.into(), text.into())]),
            ),
            &[":kotlin/main"],
        );
        assert!(
            !evidence
                .observations
                .values()
                .any(|observation| observation.kind == "CALL_RELATION")
        );
        assert!(evidence.sources.is_empty());
        assert!(
            evidence
                .boundaries
                .iter()
                .any(|boundary| boundary == "KOTLIN_CALL_SOURCE_UNAVAILABLE")
        );

        let source_text = "fun endpoint() { val s = \"🙂\"; api.pick() }";
        let call_start = source_text.find("api.pick").unwrap() as u64;
        let emoji_start = source_text.find("🙂").unwrap() as u64;
        let source_len = source_text.len() as u64;
        let sources = compile_sources(
            vec![(
                ":kotlin/main",
                BTreeMap::from([(file.into(), source_text.into())]),
                false,
            )],
            BTreeMap::new(),
        );
        for (start, end) in [
            (call_start, call_start),
            (source_len, source_len + 1),
            (emoji_start + 1, emoji_start + 2),
        ] {
            let relation = kotlin_call_relation_fact(
                &scope,
                owner,
                target,
                file,
                start,
                end,
                "bad-range-binding",
            );
            assert!(
                project_scoped(
                    &service,
                    &"a".repeat(40),
                    &digest(&service).unwrap(),
                    "DEVELOPMENT",
                    "PARTIAL",
                    vec![relation],
                    &sources,
                    &known(&[":kotlin/main"]),
                )
                .is_err(),
                "range {start}..{end} must fail closed"
            );
        }
    }

    #[test]
    fn kotlin_call_site_line_end_excludes_a_trailing_lf_or_crlf_boundary() {
        let service = projection_service();
        let file = "src/main/kotlin/example/Multiline.kt";
        let scope = json!({"compilation":":kotlin/main"});
        let owner = "callable:example/Service.endpoint#jvm:()V";
        let target = "callable:example/Api.pick#jvm:()V";
        for (index, newline) in ["\n", "\r\n"].into_iter().enumerate() {
            let text = format!(
                "fun endpoint() {{{newline}  api.pick({newline}    \"x\"){newline}}}{newline}"
            );
            let call_text = format!("api.pick({newline}    \"x\")");
            let start = text.find(&call_text).unwrap();
            let end = start + call_text.len() + newline.len();
            let binding = format!("multiline-binding-{index}");
            let fact = kotlin_call_relation_fact(
                &scope,
                owner,
                target,
                file,
                start as u64,
                end as u64,
                &binding,
            );
            let evidence = project_scoped_ok(
                &service,
                vec![fact],
                &compile_sources(
                    vec![(
                        ":kotlin/main",
                        BTreeMap::from([(file.into(), text.clone())]),
                        true,
                    )],
                    BTreeMap::new(),
                ),
                &[":kotlin/main"],
            );
            let relation = evidence
                .observations
                .values()
                .find(|observation| observation.kind == "CALL_RELATION")
                .unwrap();
            let source = &evidence.sources[&relation.source_ids[0]];
            assert_eq!(source.text, &text[start..end]);
            assert_eq!((source.start_line, source.end_line), (2, 3));
            assert_eq!(relation.normalized["callSite"]["startLine"], 2);
            assert_eq!(relation.normalized["callSite"]["endLine"], 3);
        }
    }

    #[test]
    fn scoped_call_relation_preserves_unknown_target_and_missing_source_gap() {
        let service = projection_service();
        let sources = compile_sources(vec![(":a/main", BTreeMap::new(), false)], BTreeMap::new());
        let unknown_target = call_relation_fact(
            &json!({"compilation":":a/main"}),
            "method:example.Service#endpoint()V",
            Value::Null,
            "src/main/java/example/Missing.java",
            19,
            "UNKNOWN",
        );
        let mut ownerless = call_relation_fact(
            &json!({"compilation":":a/main"}),
            "method:example.Service#endpoint()V",
            json!("method:example.Service#helper()V"),
            "src/main/java/example/Missing.java",
            20,
            "COMPILER_EXACT",
        );
        ownerless.0["sourceIdentity"] = Value::Null;
        let facts = vec![unknown_target, ownerless];
        let evidence = project_scoped_ok(&service, facts, &sources, &[":a/main"]);
        let relation = evidence
            .observations
            .values()
            .find(|observation| {
                observation.kind == "CALL_RELATION"
                    && observation.normalized["resolution"] == "UNKNOWN"
            })
            .expect("the unresolved relation fact itself remains inspectable");
        assert_eq!(relation.normalized["targetIdentity"], Value::Null);
        assert_eq!(relation.normalized["resolution"], "UNKNOWN");
        assert_eq!(
            relation.normalized["callSite"]["sourceStatus"],
            "SOURCE_UNAVAILABLE"
        );
        assert!(relation.source_ids.is_empty());
        assert!(evidence.sources.is_empty());
        let ownerless = evidence
            .observations
            .values()
            .find(|observation| {
                observation.kind == "CALL_RELATION"
                    && observation.normalized["callSite"]["ownerStatus"]
                        == "SOURCE_OWNER_UNAVAILABLE"
            })
            .expect("ownerless relation has an explicit ownership gap");
        assert_eq!(ownerless.symbol, "");
        assert!(
            evidence
                .boundaries
                .iter()
                .any(|boundary| boundary == "CALL_RELATION_OWNER_UNAVAILABLE")
        );
    }

    #[test]
    #[ignore = "qualification launches the real JDK compiler variable producer"]
    fn javac_variables_resolve_storage_modes_spans_and_scoped_admission() {
        use crate::java_adapter_v2::build_java_compiler_index;
        use crate::java_project_model::{JavaBuildSystem, JavaOperationalModel, JavaProjectModel};
        use std::process::Command;
        let original = r#"package example;
class Variables {
    int value; int attempts;
    void probe(int value, int[] arr, Variables other) {
        String café = "🙂";
        value = 3;
        this.value = value;
        this.value += other.value;
        ++value;
        arr[value] = this.value;
        { int same = value; this.value = same; }
        { int same = this.value; same++; }
        Runnable deferred = () -> { this.value = 999; };
        class Deferred { void later() { Variables.this.value = 888; } }
        System.out.println(café); attempts = attempts + 1; int repeated = value + value;
    }
    void localShadow() { int value = 1; this.value = value; }
}
"#;
        fn produce(root: &Path, source: &str) -> Vec<(Value, String)> {
            let file = "src/main/java/example/Variables.java";
            let path = root.join(file);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, source).unwrap();
            let version = Command::new("javac").arg("-version").output().unwrap();
            assert!(version.status.success());
            let compiler_version = format!(
                "{}{}",
                String::from_utf8_lossy(&version.stdout),
                String::from_utf8_lossy(&version.stderr)
            )
            .trim()
            .to_string();
            let mut authority = JavaProjectModel {
                schema: crate::java_project_model::JAVA_MODEL_SCHEMA.into(),
                model_digest: String::new(),
                build_system: JavaBuildSystem::Maven,
                compilation: ":/main".into(),
                source_files: vec![file.into()],
                classpath: vec![],
                dependency_sources: vec![],
                release: 17,
                compiler_version,
                compiler_options: vec![],
                annotation_processors: vec![],
                annotation_processor_paths: vec![],
                boundaries: vec![],
            };
            authority.model_digest = canonical::hash(&authority).unwrap();
            let operational = JavaOperationalModel {
                authority,
                source_paths: vec![path],
                classpath_paths: vec![],
                annotation_processor_paths: vec![],
                java_executable: "java".into(),
            };
            let index = build_java_compiler_index(
                root,
                &operational,
                &BTreeMap::from([(file.into(), canonical::hash_bytes(source.as_bytes()))]),
                false,
                None,
                &[],
                None,
            )
            .unwrap();
            index
                .facts
                .iter()
                .map(|fact| {
                    let mut value = serde_json::to_value(fact).unwrap();
                    let binding = java_fact_binding(fact);
                    value["scope"] = json!({"compilation":":/main"});
                    (value, binding)
                })
                .collect()
        }
        let temp = tempfile::tempdir().unwrap();
        let facts = produce(temp.path(), original);
        let file = "src/main/java/example/Variables.java";
        let variable: Vec<_> = facts
            .iter()
            .filter(|(f, _)| {
                matches!(
                    f["kind"].as_str(),
                    Some("VARIABLE_ACCESS" | "VARIABLE_DECLARATION")
                )
            })
            .map(|(f, _)| f)
            .collect();
        assert!(
            !variable.is_empty(),
            "real compiler must produce variable facts"
        );
        let declarations: Vec<_> = variable
            .iter()
            .filter(|f| f["kind"] == "VARIABLE_DECLARATION")
            .collect();
        let parameter = declarations
            .iter()
            .find(|f| f["name"] == "value" && f["variableKind"] == "PARAMETER")
            .unwrap();
        assert_eq!(parameter["variableKind"], "PARAMETER");
        assert!(
            parameter["variableIdentity"]
                .as_str()
                .unwrap()
                .ends_with("/slot/0")
        );
        let local_shadow = declarations
            .iter()
            .find(|f| f["name"] == "value" && f["variableKind"] == "LOCAL_VARIABLE")
            .unwrap();
        assert_ne!(
            local_shadow["variableIdentity"],
            parameter["variableIdentity"]
        );
        assert!(facts.iter().any(|(f, _)| f["kind"] == "DECLARATION"
            && f["declarationKind"] == "FIELD"
            && f["symbolIdentity"] == "field:class:example.Variables#value:I"));
        let sibling_ids: BTreeSet<_> = declarations
            .iter()
            .filter(|f| f["name"] == "same")
            .map(|f| f["variableIdentity"].as_str().unwrap())
            .collect();
        assert_eq!(
            sibling_ids.len(),
            2,
            "same spelling in sibling scopes is different storage"
        );
        let accesses: Vec<_> = variable
            .iter()
            .filter(|f| f["kind"] == "VARIABLE_ACCESS")
            .collect();
        let span = |f: &Value| {
            &original
                [f["byteStart"].as_u64().unwrap() as usize..f["byteEnd"].as_u64().unwrap() as usize]
        };
        assert!(
            !accesses
                .iter()
                .any(|f| f["name"] == "café" && f["startLine"] == 5),
            "initializer definition must not be an identifier read"
        );
        assert!(accesses.iter().any(|f| span(f) == "this.value"
            && f["accessMode"] == "WRITE"
            && f["variableIdentity"] == "field:class:example.Variables#value:I"));
        assert!(
            accesses
                .iter()
                .any(|f| span(f) == "this.value" && f["accessMode"] == "READ_WRITE")
        );
        assert!(accesses.iter().any(|f| span(f) == "value"
            && f["accessMode"] == "WRITE"
            && f["variableIdentity"] == parameter["variableIdentity"]));
        assert!(
            accesses
                .iter()
                .any(|f| span(f) == "value" && f["accessMode"] == "READ_WRITE")
        );
        assert!(
            accesses
                .iter()
                .filter(|f| f["name"] == "arr")
                .all(|f| f["accessMode"] == "READ")
        );
        assert!(
            accesses
                .iter()
                .any(|f| span(f) == "other" && f["accessMode"] == "READ")
        );
        assert!(
            accesses
                .iter()
                .any(|f| span(f) == "other.value" && f["accessMode"] == "READ")
        );
        assert!(accesses.iter().any(|f| span(f) == "café"
            && f["byteStart"].as_u64().unwrap() > f["start"].as_u64().unwrap()));
        assert!(accesses.iter().any(|f| f["variableIdentity"]
            == "field:class:java.lang.System#out:Ljava/io/PrintStream;"
            && f["declarationStatus"] == "DECLARATION_SOURCE_UNAVAILABLE"));
        assert!(
            !accesses
                .iter()
                .any(|f| f["startLine"] == 13 || f["startLine"] == 14),
            "deferred bodies must not emit immediate accesses"
        );
        for code in [
            "JAVA_VARIABLE_LAMBDA_DEFERRED",
            "JAVA_VARIABLE_LOCAL_CLASS_DEFERRED",
        ] {
            assert!(
                facts
                    .iter()
                    .any(|(f, _)| f["kind"] == "BOUNDARY" && f["code"] == code)
            );
        }
        assert!(
            declarations
                .iter()
                .any(|f| f["name"] == "café" && f["definitionKind"] == "INITIALIZER_DEFINITION")
        );
        let tables = compile_sources(
            vec![(
                ":/main",
                BTreeMap::from([(file.into(), original.into())]),
                false,
            )],
            BTreeMap::new(),
        );
        let known = BTreeSet::from([":/main".into()]);
        let evidence =
            project_scoped_ok(&projection_service(), facts.clone(), &tables, &[":/main"]);
        let rows: Vec<_> = evidence
            .observations
            .values()
            .filter(|o| o.kind == "VARIABLE_ACCESS")
            .collect();
        assert_eq!(rows.len(), accesses.len());
        for row in &rows {
            let site = &row.normalized["variableSite"];
            let start = site["byteStart"].as_u64().unwrap() as usize;
            let end = site["byteEnd"].as_u64().unwrap() as usize;
            assert_eq!(
                site["spanDigest"],
                canonical::hash_bytes(&original.as_bytes()[start..end])
            );
            assert_eq!(
                site["sourceContentDigest"],
                canonical::hash_bytes(original.as_bytes())
            );
            assert!(evidence.sources.contains_key(&row.source_ids[0]));
            assert!(
                evidence
                    .observations
                    .contains_key(row.normalized["callableObservationId"].as_str().unwrap())
            );
            if let Some(target) = row.normalized["declarationObservationId"].as_str() {
                assert!(evidence.observations.contains_key(target));
            }
        }
        let checked = super::super::check::Check {
            schema: "codeclew-documentation-check/1.0".into(),
            input_digest: digest(&"owned-variable-fixture").unwrap(),
            context_digest: digest(&evidence).unwrap(),
            services: BTreeMap::from([("svc".into(), evidence.clone())]),
            unresolved: BTreeMap::new(),
            interactions: BTreeMap::new(),
            scenarios: BTreeMap::new(),
            dependencies: evidence.observations.clone(),
            source_inputs: None,
            composition: None,
        };
        let args = super::super::cli::ContextArgs {
            root: temp.path().to_path_buf(),
            service: Some("svc".into()),
            scenario: None,
            entrypoint: None,
            symbols: vec![],
            source_ids: vec![],
            dependency_ids: vec![rows[0].id.clone()],
            format: super::super::cli::ContextFormat::Raw,
            refresh: false,
            snapshot: None,
            cursor: None,
            limit: 100,
        };
        let raw = super::super::cli::context_items(&checked, &args, None).unwrap();
        assert!(
            raw.iter().any(
                |row| row["kind"] == "DEPENDENCY" && row["record"]["kind"] == "VARIABLE_ACCESS"
            )
        );
        assert!(
            raw.iter()
                .any(|row| row["kind"] == "SOURCE" && row["id"] == rows[0].source_ids[0])
        );
        let mut attempts: Vec<_> = rows
            .iter()
            .filter(|o| o.normalized["name"] == "attempts")
            .copied()
            .collect();
        attempts.sort_by_key(|o| o.normalized["variableSite"]["byteStart"].as_u64().unwrap());
        assert_eq!(attempts.len(), 2);
        assert_eq!(attempts[0].normalized["accessMode"], "WRITE");
        assert_eq!(attempts[1].normalized["accessMode"], "READ");
        assert_eq!(
            attempts[0].normalized["variableIdentity"],
            attempts[1].normalized["variableIdentity"]
        );
        let mut repeated: Vec<_> = rows
            .iter()
            .filter(|o| {
                o.normalized["name"] == "value" && o.normalized["variableSite"]["startLine"] == 15
            })
            .copied()
            .collect();
        repeated.sort_by_key(|o| o.normalized["variableSite"]["byteStart"].as_u64().unwrap());
        assert_eq!(repeated.len(), 2);
        let first = original.rfind("value + value").unwrap();
        assert_eq!(repeated[0].normalized["variableSite"]["byteStart"], first);
        assert_eq!(
            repeated[1].normalized["variableSite"]["byteStart"],
            first + 8
        );
        assert_ne!(repeated[0].id, repeated[1].id);
        assert_eq!(
            repeated[0].normalized["variableIdentity"],
            repeated[1].normalized["variableIdentity"]
        );
        for row in attempts.iter().chain(repeated.iter()) {
            let site = &row.normalized["variableSite"];
            let source = &evidence.sources[&row.source_ids[0]];
            let source_start = site["sourceByteStart"].as_u64().unwrap() as usize;
            let source_end = site["sourceByteEnd"].as_u64().unwrap() as usize;
            let start = site["byteStart"].as_u64().unwrap() as usize - source_start;
            let end = site["byteEnd"].as_u64().unwrap() as usize - source_start;
            assert_eq!(&original[source_start..source_end], source.text);
            assert_eq!(
                &source.text[start..end],
                row.normalized["name"].as_str().unwrap()
            );
            assert_eq!(
                site["spanDigest"],
                canonical::hash_bytes(&source.text.as_bytes()[start..end])
            );
        }
        let site_id = attempts[0].id.clone();
        let site = &attempts[0].normalized["variableSite"];
        let inside_unicode = site["sourceByteStart"].as_u64().unwrap()
            + evidence.sources[&attempts[0].source_ids[0]]
                .text
                .find('é')
                .unwrap() as u64
            + 1;
        for (key, value) in [
            ("byteStart", json!(inside_unicode)),
            (
                "sourceByteStart",
                json!(site["sourceByteStart"].as_u64().unwrap() + 1),
            ),
            (
                "sourceByteEnd",
                json!(site["sourceByteEnd"].as_u64().unwrap() - 1),
            ),
            (
                "byteStart",
                json!(site["sourceByteStart"].as_u64().unwrap() - 1),
            ),
            (
                "byteEnd",
                json!(site["sourceByteEnd"].as_u64().unwrap() + 1),
            ),
            ("sourceByteStart", Value::Null),
            ("sourceByteEnd", json!("not-a-byte-bound")),
            ("sourceId", json!("missing")),
            ("file", json!("Other.java")),
            ("service", json!("other")),
            ("revision", json!("0".repeat(40))),
            ("sourceDigest", json!("sha256:forged")),
            ("spanDigest", json!("sha256:forged")),
            ("evidenceDigest", json!("sha256:forged")),
        ] {
            let mut bad = evidence.clone();
            let observation = bad.observations.get_mut(&site_id).unwrap();
            observation.normalized["variableSite"][key] = value;
            observation.digest = digest(&observation.normalized).unwrap();
            assert!(verify_evidence(&bad).is_err(), "portable forged {key}");
        }
        let mut partial = evidence.clone();
        let observation = partial.observations.get_mut(&site_id).unwrap();
        observation.normalized["variableSite"]
            .as_object_mut()
            .unwrap()
            .remove("sourceByteEnd");
        observation.digest = digest(&observation.normalized).unwrap();
        assert!(
            verify_evidence(&partial).is_err(),
            "partial bounds are never legacy absence"
        );
        let mut legacy = evidence.clone();
        let observation = legacy.observations.get_mut(&site_id).unwrap();
        for key in ["sourceByteStart", "sourceByteEnd", "service", "revision"] {
            observation.normalized["variableSite"]
                .as_object_mut()
                .unwrap()
                .remove(key);
        }
        observation.digest = digest(&observation.normalized).unwrap();
        verify_evidence(&legacy).unwrap();
        // Mutations are admission negatives of actual compiler output, not manufactured compiler proof.
        let access_index = facts
            .iter()
            .position(|(f, _)| f["kind"] == "VARIABLE_ACCESS" && f["variableKind"] == "PARAMETER")
            .unwrap();
        for (key, value) in [
            ("byteEnd", json!(original.len() + 1)),
            ("byteStart", json!(original.find('é').unwrap() + 1)),
            (
                "enclosingCallable",
                json!("method:class:example.Other#probe()V"),
            ),
            (
                "variableIdentity",
                json!("parameter:method:class:example.Other#probe(I)V/slot/0"),
            ),
            ("declarationStatus", json!("DECLARATION_SOURCE_UNAVAILABLE")),
        ] {
            let mut bad = facts.clone();
            bad[access_index].0[key] = value;
            let mut raw = bad[access_index].0.clone();
            raw.as_object_mut().unwrap().remove("scope");
            bad[access_index].1 = java_fact_binding(&raw);
            assert!(
                validate_variable_facts(&bad, &tables, &known).is_err(),
                "{key}"
            );
        }
        let mut semantic_binding = facts.clone();
        let mut raw = semantic_binding[access_index].0.clone();
        raw.as_object_mut().unwrap().remove("scope");
        semantic_binding[access_index].1 = digest(&raw).unwrap();
        assert_ne!(semantic_binding[access_index].1, facts[access_index].1);
        assert!(
            validate_variable_facts(&semantic_binding, &tables, &known)
                .unwrap_err()
                .message
                .contains("compiler variable payload binding differs"),
            "an unsalted semantic digest cannot replace the actual CAS reference"
        );
        let mut bad_binding = facts.clone();
        bad_binding[access_index].1 = "sha256:forged".into();
        assert!(validate_variable_facts(&bad_binding, &tables, &known).is_err());
        let mut duplicate = facts.clone();
        duplicate.push(facts[access_index].clone());
        assert!(validate_variable_facts(&duplicate, &tables, &known).is_err());
        let mut wrong_scope = facts.clone();
        wrong_scope[access_index].0["scope"] = json!({"compilation":":other/main"});
        assert!(validate_variable_facts(&wrong_scope, &tables, &known).is_err());
        let mut cross_scope = facts.clone();
        cross_scope[access_index].0["scope"] = json!({"compilation":":other/main"});
        for (fact, binding) in &facts {
            if fact["kind"] == "SOURCE_FILE"
                || (fact["kind"] == "DECLARATION" && fact["declarationKind"] == "METHOD")
            {
                let mut copied = fact.clone();
                copied["scope"] = json!({"compilation":":other/main"});
                cross_scope.push((copied, binding.clone()));
            }
        }
        let two_scopes = compile_sources(
            vec![
                (
                    ":/main",
                    BTreeMap::from([(file.into(), original.into())]),
                    false,
                ),
                (
                    ":other/main",
                    BTreeMap::from([(file.into(), original.into())]),
                    false,
                ),
            ],
            BTreeMap::new(),
        );
        assert!(
            validate_variable_facts(
                &cross_scope,
                &two_scopes,
                &BTreeSet::from([":/main".into(), ":other/main".into()])
            )
            .is_err(),
            "registered scope must not borrow parameter target from another scope"
        );
        let wrong_tables = compile_sources(
            vec![(
                ":/main",
                BTreeMap::from([(file.into(), original.replace("value = 3", "value = 4"))]),
                false,
            )],
            BTreeMap::new(),
        );
        assert!(validate_variable_facts(&facts, &wrong_tables, &known).is_err());
        // A second genuine compiler capture exercises constructor formals,
        // qualified/static fields and several owners, as in native linked pages.
        let task_probe = r#"package example;
class Task {
    static final int DEFAULT_PRIORITY = 1;
    String name;
    int priority;
    Task(String name, int priority) { this.name = name; this.priority = priority; }
}
class UseTask {
    String prepare(Task task) {
        int priority = Task.DEFAULT_PRIORITY;
        String name = task.name;
        return name + priority;
    }
}
"#;
        let task_facts = produce(temp.path(), task_probe);
        let task_tables = compile_sources(
            vec![(
                ":/main",
                BTreeMap::from([(file.into(), task_probe.into())]),
                false,
            )],
            BTreeMap::new(),
        );
        let task_evidence = project_scoped_ok(
            &projection_service(),
            task_facts.clone(),
            &task_tables,
            &[":/main"],
        );
        verify_evidence(&task_evidence).unwrap();
        let task_variables = task_facts
            .iter()
            .filter(|(f, _)| {
                matches!(
                    f["kind"].as_str(),
                    Some("VARIABLE_ACCESS" | "VARIABLE_DECLARATION")
                )
            })
            .collect::<Vec<_>>();
        assert!(
            task_variables
                .iter()
                .any(|(f, _)| f["kind"] == "VARIABLE_DECLARATION"
                    && f["variableKind"] == "PARAMETER"
                    && f["enclosingCallable"].as_str().unwrap().contains("#<init>"))
        );
        for exact in [
            "this.name",
            "this.priority",
            "Task.DEFAULT_PRIORITY",
            "task.name",
        ] {
            assert!(
                task_variables
                    .iter()
                    .any(|(f, _)| f["kind"] == "VARIABLE_ACCESS"
                        && &task_probe[f["byteStart"].as_u64().unwrap() as usize
                            ..f["byteEnd"].as_u64().unwrap() as usize]
                            == exact),
                "{exact}"
            );
        }
        for (fact, binding) in task_variables {
            let mut raw = fact.clone();
            raw.as_object_mut().unwrap().remove("scope");
            assert_eq!(*binding, java_fact_binding(&raw));
            assert_ne!(*binding, digest(&raw).unwrap());
            let rows = task_evidence
                .observations
                .values()
                .filter(|o| {
                    o.kind == fact["kind"].as_str().unwrap()
                        && o.normalized["variableSite"]["byteStart"] == fact["byteStart"]
                        && o.normalized["variableSite"]["byteEnd"] == fact["byteEnd"]
                        && o.normalized["variableIdentity"] == fact["variableIdentity"]
                        && o.normalized["occurrencePath"] == fact["occurrencePath"]
                        && o.normalized["enclosingCallable"] == fact["enclosingCallable"]
                })
                .collect::<Vec<_>>();
            assert_eq!(
                rows.len(),
                1,
                "exact variable occurrence must remain unique"
            );
            let row = rows[0];
            assert_eq!(row.normalized["variableSite"]["evidenceDigest"], *binding);
            assert_eq!(
                task_evidence.sources[&row.source_ids[0]].evidence_digest,
                *binding
            );
        }
        let relocated = format!("// unrelated file line relocation 🙂\n\n{original}");
        let relocated_facts = produce(temp.path(), &relocated);
        let identities = |fs: &[(Value, String)]| {
            fs.iter()
                .filter(|(f, _)| {
                    matches!(
                        f["kind"].as_str(),
                        Some("VARIABLE_ACCESS" | "VARIABLE_DECLARATION")
                    )
                })
                .map(|(f, _)| {
                    (
                        f["kind"].to_string(),
                        f["variableIdentity"].to_string(),
                        f["occurrencePath"].to_string(),
                        f["accessMode"].to_string(),
                    )
                })
                .collect::<BTreeSet<_>>()
        };
        assert_eq!(identities(&facts), identities(&relocated_facts));
        // These are new real compiler captures, not coordinate-only or injected fact probes.
        let multiline = original.replace(
            "String café = \"🙂\";",
            "String café =\n            \"🙂\";",
        );
        let mixed = multiline
            .lines()
            .enumerate()
            .map(|(index, line)| format!("{line}{}", ["\r", "\r\n", "\n"][index % 3]))
            .collect::<String>();
        for (name, probe) in [
            ("CR", multiline.replace('\n', "\r")),
            ("CRLF", multiline.replace('\n', "\r\n")),
            ("mixed", mixed),
        ] {
            let probe_facts = produce(temp.path(), &probe);
            assert_eq!(
                identities(&facts),
                identities(&probe_facts),
                "{name} changes source coordinates, not storage/occurrence identity"
            );
            let probe_tables = compile_sources(
                vec![(
                    ":/main",
                    BTreeMap::from([(file.into(), probe.clone())]),
                    false,
                )],
                BTreeMap::new(),
            );
            let probe_evidence = project_scoped_ok(
                &projection_service(),
                probe_facts.clone(),
                &probe_tables,
                &[":/main"],
            );
            verify_evidence(&probe_evidence).unwrap();
            let lines = javac_line_ranges(&probe);
            let mut retained_multiline = false;
            for row in probe_evidence
                .observations
                .values()
                .filter(|o| matches!(o.kind.as_str(), "VARIABLE_ACCESS" | "VARIABLE_DECLARATION"))
            {
                let site = &row.normalized["variableSite"];
                let source = &probe_evidence.sources[&row.source_ids[0]];
                let expected = &probe[lines[source.start_line as usize - 1].start
                    ..lines[source.end_line as usize - 1].end];
                assert_eq!(
                    source.text, expected,
                    "{name}: retained context must preserve original CR/CRLF bytes"
                );
                let start = site["byteStart"].as_u64().unwrap() as usize;
                let end = site["byteEnd"].as_u64().unwrap() as usize;
                assert_eq!(
                    site["spanDigest"],
                    canonical::hash_bytes(&probe.as_bytes()[start..end])
                );
                assert_eq!(
                    site["sourceContentDigest"],
                    canonical::hash_bytes(probe.as_bytes())
                );
                let context_start = site["sourceByteStart"].as_u64().unwrap() as usize;
                let context_end = site["sourceByteEnd"].as_u64().unwrap() as usize;
                assert_eq!(&probe[context_start..context_end], source.text);
                assert_eq!(
                    site["spanDigest"],
                    canonical::hash_bytes(
                        &source.text.as_bytes()[start - context_start..end - context_start]
                    )
                );
                if name == "CRLF"
                    && row.kind == "VARIABLE_ACCESS"
                    && row.normalized["name"] == "attempts"
                {
                    let column = start - context_start;
                    assert!(source.text[..column].contains("café"));
                    assert!(
                        column > source.text[..column].encode_utf16().count(),
                        "Unicode byte columns are not UTF16 columns"
                    );
                    assert_eq!(&source.text[column..end - context_start], "attempts");
                }
                if row.kind == "VARIABLE_DECLARATION" && row.normalized["name"] == "café" {
                    assert_eq!(source.end_line - source.start_line, 1);
                    assert!(source.text.contains('\r'));
                    retained_multiline = true;
                }
            }
            assert!(
                retained_multiline,
                "{name}: multiline variable declaration was not captured"
            );
            let mut wrong_lines = probe_facts;
            let index = wrong_lines
                .iter()
                .position(|(f, _)| f["kind"] == "VARIABLE_ACCESS")
                .unwrap();
            wrong_lines[index].0["startLine"] = json!(1);
            let mut raw = wrong_lines[index].0.clone();
            raw.as_object_mut().unwrap().remove("scope");
            wrong_lines[index].1 = java_fact_binding(&raw);
            assert!(
                validate_variable_facts(&wrong_lines, &probe_tables, &known).is_err(),
                "{name}: LF-style/forged compiler line must fail"
            );
        }
        let relocated_tables = compile_sources(
            vec![(":/main", BTreeMap::from([(file.into(), relocated)]), false)],
            BTreeMap::new(),
        );
        let relocated_evidence = project_scoped_ok(
            &projection_service(),
            relocated_facts,
            &relocated_tables,
            &[":/main"],
        );
        assert_eq!(
            rows.iter().map(|o| o.id.clone()).collect::<BTreeSet<_>>(),
            relocated_evidence
                .observations
                .values()
                .filter(|o| o.kind == "VARIABLE_ACCESS")
                .map(|o| o.id.clone())
                .collect()
        );
    }

    #[test]
    fn javac_try_invocation_projects_as_citable_call_relation_while_flow_stays_bounded() {
        use crate::java_adapter_v2::{JavaCompilerFact, build_java_compiler_index};
        use crate::java_project_model::{JavaBuildSystem, JavaOperationalModel, JavaProjectModel};
        use std::process::Command;

        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path();
        let file = "src/main/java/example/Service.java";
        let source = "package example;\nclass Service {\n  void endpoint() { try { helper(); } catch (RuntimeException ex) {} }\n  void helper() {}\n}\n";
        let path = root.join(file);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, source).unwrap();
        let version = Command::new("javac").arg("-version").output().unwrap();
        let version_text = format!(
            "{}{}",
            String::from_utf8_lossy(&version.stdout),
            String::from_utf8_lossy(&version.stderr)
        );
        let compiler_version = version_text
            .lines()
            .find(|line| line.starts_with("javac "))
            .unwrap()
            .to_owned();
        let mut authority = JavaProjectModel {
            schema: crate::java_project_model::JAVA_MODEL_SCHEMA.into(),
            model_digest: String::new(),
            build_system: JavaBuildSystem::Maven,
            compilation: ":/main".into(),
            source_files: vec![file.into()],
            classpath: Vec::new(),
            dependency_sources: Vec::new(),
            release: 17,
            compiler_version,
            compiler_options: Vec::new(),
            annotation_processors: Vec::new(),
            annotation_processor_paths: Vec::new(),
            boundaries: Vec::new(),
        };
        authority.model_digest = crate::canonical::hash(&authority).unwrap();
        let operational = JavaOperationalModel {
            authority,
            source_paths: vec![path],
            classpath_paths: Vec::new(),
            annotation_processor_paths: Vec::new(),
            java_executable: "java".into(),
        };
        let content_digests =
            BTreeMap::from([(file.into(), crate::canonical::hash_bytes(source.as_bytes()))]);
        let index =
            build_java_compiler_index(root, &operational, &content_digests, false, None, &[], None)
                .unwrap();
        let endpoint = index
            .facts
            .iter()
            .find_map(|fact| match fact {
                JavaCompilerFact::Declaration {
                    name: Some(name),
                    symbol_identity,
                    ..
                } if name == "endpoint" => Some(symbol_identity.clone()),
                _ => None,
            })
            .unwrap();
        let relation = index
            .facts
            .iter()
            .find_map(|fact| match fact {
                JavaCompilerFact::Relation {
                    relation_kind,
                    source_identity,
                    target_identity,
                    start_line: Some(3),
                    ..
                } if relation_kind == "CALLS" && source_identity == &endpoint => {
                    Some(target_identity.clone())
                }
                _ => None,
            })
            .expect("the general javac scanner sees helper() inside try");
        let scope = json!({"compilation":":/main"});
        let facts = index
            .facts
            .iter()
            .map(|fact| {
                let mut value = serde_json::to_value(fact).unwrap();
                value["scope"] = scope.clone();
                (value, java_fact_binding(fact))
            })
            .collect();
        let table = BTreeMap::from([(file.into(), source.into())]);
        let sources = compile_sources(vec![(":/main", table, true)], BTreeMap::new());
        let service = projection_service();
        let evidence = project_scoped_ok(&service, facts, &sources, &[":/main"]);
        let call = evidence
            .observations
            .values()
            .find(|observation| {
                observation.kind == "CALL_RELATION"
                    && observation.normalized["sourceIdentity"] == endpoint
                    && observation.normalized["targetIdentity"] == relation
            })
            .expect("javac call relation survives scoped projection");
        assert_eq!(call.normalized["relationKind"], "CALLS");
        assert_eq!(call.normalized["resolution"], "COMPILER_EXACT");
        assert!(
            evidence.sources[&call.source_ids[0]]
                .text
                .contains("helper();")
        );
        assert!(
            evidence
                .boundaries
                .iter()
                .any(|boundary| { boundary == "EXCEPTION_FLOW_REQUIRES_SOURCE_REVIEW" })
        );
        assert!(evidence.observations.values().any(|observation| {
            observation.kind == "FLOW"
                && observation.symbol == endpoint
                && observation.normalized["kind"] == "BOUNDARY"
        }));
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
