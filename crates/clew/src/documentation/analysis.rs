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
    path::{Path, PathBuf},
    process::Stdio,
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
        let mut source = super::syntax::capture(service, &repo)?;
        if let Some(semantic) = semantic {
            let mut provider = service.clone();
            provider.source = None;
            provider.modules = None;
            provider.profile = semantic.profile.clone();
            provider.compilation = semantic.compilation.clone();
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
    eprintln!("CODEDEBUG doctor START for {}", service.id);
    let readiness = operations::doctor(
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
    )?;
    if readiness["status"] != "PASS" {
        let action = readiness["nextAction"]
            .as_str()
            .unwrap_or("RUN_TASK_DOCTOR");
        return Err(ClewError::new(
            ErrorCode::PreconditionFailed,
            format!("documentation source admission requires action: {action}"),
        ));
    }
    eprintln!("CODEDEBUG doctor PASS for {}", service.id);
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
    let session = SessionAuthority::open(
        &repo,
        &service.target_ref,
        language,
        &compilations,
        None,
        ModelCachePolicy::NonCacheable,
        None,
    )?;
    let result = capture_session(&session, service, &service_digest, debug_output);
    // Only use supported lifecycle operations; documentation records have no session dependency.
    let cleanup = session.abort().and_then(|_| session.gc(false)).map(|_| ());
    let mut evidence = match result {
        Ok(evidence) => evidence,
        Err(error) => {
            eprintln!("CODEDEBUG capture_session ERR for {}: {error}", service.id);
            cleanup?;
            return Err(error);
        }
    };
    eprintln!(
        "CODEDEBUG capture_session OK for {} ({} sources, {} observations)",
        service.id,
        evidence.sources.len(),
        evidence.observations.len()
    );
    super::contracts::capture(service, &repo, &mut evidence)?;
    eprintln!("CODEDEBUG contracts OK for {}", service.id);
    super::modules::attach(service, &mut evidence)?;
    eprintln!("CODEDEBUG modules OK for {}", service.id);
    cleanup?;
    if git(&repo, &["rev-parse", "--verify", "HEAD^{commit}"])? != revision {
        return Err(ClewError::new(
            ErrorCode::InputMutated,
            "service revision changed during documentation extraction",
        ));
    }
    let _lock = repository.lock()?;
    // New-format captures persist a small reference envelope: the heavy
    // payload lives once in the immutable object store, and the keyed cache
    // path holds validated object references instead of a duplicated full
    // ServiceEvidence serialization.
    let mut manifest = super::cache::store_capture(repository, &evidence)?;
    // Maven/external-state capture has no complete build/settings/dependency
    // authority in the reuse key, so it is explicitly non-cacheable (recapture
    // on the next run) rather than silently reusable.
    super::cache::mark_non_cacheable(
        &mut manifest,
        "Maven/external-state capture lacks complete build/settings/dependency authority",
    );
    repository.atomic(&cache_path, &bytes(&manifest)?)?;
    eprintln!("CODEDEBUG cache write OK for {}", service.id);
    Ok(evidence)
}

fn capture_session(
    session: &SessionAuthority,
    service: &Service,
    service_digest: &str,
    debug_output: Option<&crate::maven_diagnostics::DebugOutput>,
) -> Result<ServiceEvidence, ClewError> {
    let ready = generation_service::ensure_session_generation_with_diagnostics(
        session,
        debug_output,
        generation_service::wants_writable_then_seal(&service.profile),
        &service.annotation_processor_paths,
    )?;
    eprintln!(
        "CODEDEBUG capture_session ensure_generation OK for {}",
        service.id
    );
    let state = StateAuthority::process_default()?;
    let store = CasStore::open(&state)?;
    // A writable-then-seal generation persists the exact transformed source
    // bytes it was indexed against. Documentation must read those bytes, never
    // slice the original repository snapshot with transformed coordinates.
    // Each admitted compilation carries its own transformed reference, so the
    // source table is keyed per compilation scope instead of sharing the
    // set-level first-present reference across all scopes.
    let snapshot = generation_service::load_snapshot(&store, &ready)?;
    eprintln!(
        "CODEDEBUG capture_session load_snapshot OK for {}",
        service.id
    );
    let mut snapshot_text: BTreeMap<String, String> = BTreeMap::new();
    for entry in &snapshot.worktree {
        if let Some(reference) = &entry.content {
            let lease = store.read(reference, store::MAX_RECORD as usize)?;
            if let Ok(text) = String::from_utf8(lease.bytes().to_vec()) {
                snapshot_text.insert(entry.path.clone(), text);
            }
        }
    }
    let mut facts = Vec::new();
    let mut count = 0usize;
    let mut total = 0u64;
    // Scope membership is attached only when more than one compilation is
    // admitted; a single-compilation service keeps the legacy (scope-free)
    // evidence so existing records round-trip and digest unchanged.
    let multi_scope = ready.compilations.len() > 1;
    for compilation in &ready.compilations {
        let lease = store.read(&compilation.generation, MAX_EVIDENCE as usize)?;
        eprintln!(
            "CODEDEBUG capture_session read generation OK for {}",
            service.id
        );
        let generation: GenerationManifest =
            serde_json::from_slice(lease.bytes()).map_err(io_error)?;
        let scope = compilation.clone();
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
            // last-write-wins overwriting. Single-scope records stay unmarked.
            if multi_scope {
                value["scope"] = json!(scope);
            }
            facts.push((value, fact.payload.digest.clone()));
            Ok(())
        })?;
        eprintln!(
            "CODEDEBUG capture_session visit_facts OK for {} (count={count})",
            service.id
        );
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
            contracts.insert(file.clone(), text.clone());
        }
    }
    let known: BTreeSet<String> = ready
        .compilations
        .iter()
        .map(|c| c.compilation.clone())
        .collect();
    // A single admitted compilation keeps the empty-scope legacy key so its
    // scope-free facts resolve to the sole source table and records round-trip.
    let mut contents = BTreeMap::new();
    let mut transformed_map = BTreeMap::new();
    let mut source_bytes = 0usize;
    eprintln!(
        "CODEDEBUG capture_session reading sources for {} (wanted={})",
        service.id,
        wanted.len()
    );
    for compilation in &ready.compilations {
        let scope = if multi_scope {
            compilation.compilation.clone()
        } else {
            String::new()
        };
        let (bytes, transformed) = match &compilation.transformed_source {
            Some(reference) => (
                generation_service::load_transformed_source(&store, reference)?,
                true,
            ),
            // Read-only generations share the original snapshot for their
            // occurrences and never claim transformed authority.
            None => (
                snapshot_text
                    .iter()
                    .map(|(k, v)| (k.clone(), v.clone().into_bytes()))
                    .collect(),
                false,
            ),
        };
        let mut table = BTreeMap::new();
        for path in &wanted {
            if let Some(bytes) = bytes.get(path)
                && let Ok(text) = std::str::from_utf8(bytes)
            {
                source_bytes = source_bytes.saturating_add(text.len());
                if source_bytes > MAX_EVIDENCE as usize {
                    return Err(ClewError::new(
                        ErrorCode::SliceBudgetExceeded,
                        "documentation source byte budget exceeded",
                    ));
                }
                table.insert(path.clone(), text.to_owned());
            }
        }
        contents.insert(scope.clone(), table);
        transformed_map.insert(scope, transformed);
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
/// both. The legacy single-compilation path uses the empty scope key.
pub(crate) struct CompilationSource {
    pub contents: BTreeMap<String, BTreeMap<String, String>>,
    pub transformed: BTreeMap<String, bool>,
    pub contracts: BTreeMap<String, String>,
}

impl CompilationSource {
    fn text(&self, scope: &str, file: &str) -> Option<&str> {
        if let Some(text) = self.contents.get(scope).and_then(|table| table.get(file)) {
            return Some(text);
        }
        if scope.is_empty() {
            return self.contracts.get(file).map(String::as_str);
        }
        None
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
    let Some(text) = sources.text(scope, file) else {
        // A transformed compilation that cannot supply a required file must fail
        // explicitly rather than silently falling back to another scope or the
        // original snapshot.
        if !scope.is_empty() && sources.transformed.get(scope).copied().unwrap_or(false) {
            return Err(invalid(format!(
                "transformed compilation {scope} lacks required source file {file}"
            )));
        }
        return Ok(None);
    };
    let transformed = sources.transformed.get(scope).copied().unwrap_or(false);
    let (Some(start), Some(end)) = (fact["startLine"].as_u64(), fact["endLine"].as_u64()) else {
        return Ok(None);
    };
    let lines: Vec<_> = text.lines().collect();
    if start == 0 || end < start || end > lines.len() as u64 {
        return Ok(None);
    }
    let exact = lines[start as usize - 1..end as usize].join("\n");
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
/// instead of colliding. An empty scope (legacy single compilation) yields the
/// unmodified identity, preserving existing records.
fn scoped_identity(scope: &str, identity: &str) -> String {
    if scope.is_empty() {
        identity.to_string()
    } else {
        format!("{scope}\u{1f}{identity}")
    }
}

/// Resolve the canonical compilation-scope key from a fact's `scope` value.
///
/// Production capture attaches the whole `ReadyGeneration` object as `scope`
/// (see `capture_session`), so an object's `compilation` field is authoritative
/// and must name a registered compilation. A nonempty legacy string scope is
/// accepted unchanged for older records. An empty scope is used only when the
/// scope is absent, which is valid solely for a single admitted compilation.
/// A malformed object or an unknown registered scope is an explicit error, never
/// a silent empty fallback.
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
        Value::String(s) if !s.is_empty() => Ok(s.clone()),
        _ => Ok(String::new()),
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
    // Legacy single-compilation projection: the flat files map becomes the
    // empty-scope source table and the contract table.
    let mut contents = BTreeMap::new();
    contents.insert(String::new(), files.clone());
    let mut transformed_map = BTreeMap::new();
    transformed_map.insert(String::new(), transformed);
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
        &BTreeSet::new(),
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
            if fact["code"].as_str() == Some("JAVA_COMPILER_DIAGNOSTIC") {
                eprintln!(
                    "CODEDEBUG JAVA_COMPILER_DIAGNOSTIC diagnosticCode={} file={} line={}",
                    fact["diagnosticCode"].as_str().unwrap_or("?"),
                    fact["file"].as_str().unwrap_or("?"),
                    fact["line"]
                );
            }
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
        // Production attaches the ReadyGeneration object as scope; its
        // `compilation` field is authoritative and yields a distinct stable key.
        let object_scope = json!({ "compilation": ":flows:lead-tinkoff-decision-flow/main" });
        assert_eq!(
            resolve_scope_key(&object_scope, &known).unwrap(),
            ":flows:lead-tinkoff-decision-flow/main"
        );
        // A nonempty legacy string scope is accepted unchanged.
        assert_eq!(
            resolve_scope_key(&json!(":common/main"), &known).unwrap(),
            ":common/main"
        );
        // Absent scope (single compilation) yields the empty legacy key.
        assert_eq!(resolve_scope_key(&Value::Null, &known).unwrap(), "");
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
            "compilation":":/main", "targetRef":"main",
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
            "compilation":":/main", "targetRef":"main",
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
            contents.insert(scope.to_string(), table);
            transformed.insert(scope.to_string(), is_transformed);
        }
        CompilationSource {
            contents,
            transformed,
            contracts,
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
    fn single_scope_legacy_empty_key_round_trips() {
        let service = projection_service();
        let file = "src/main/java/example/Service.java";
        let mut table = BTreeMap::new();
        table.insert(
            file.into(),
            "package example;\npublic class Service {}\n".into(),
        );
        // A single admitted compilation uses the empty-scope legacy key, so a
        // scope-free fact (scope absent) resolves to the sole source table.
        let sources = compile_sources(vec![("", table, true)], BTreeMap::new());
        let evidence = project_scoped_ok(
            &service,
            vec![declaration_fact(&Value::Null, "example.Service", file, 2)],
            &sources,
            &[],
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
            "compilation":":/main", "targetRef":"main",
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
