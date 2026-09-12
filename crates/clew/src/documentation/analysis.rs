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

const MAX_EVIDENCE: u64 = 64 * 1024 * 1024;
const MAX_FACTS: usize = 131_072;

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
    let repo = bound_repository(repository, service)?;
    let runtime = RuntimeAuthority::from_environment()?
        .ok_or_else(|| invalid("documentation analysis requires the supported clew launcher"))?;
    let compilations = vec![service.compilation.clone()];
    let language = if service.language == "kotlin" {
        SessionLanguage::Kotlin
    } else {
        SessionLanguage::Java
    };
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
    let result = capture_session(&session, service, &service_digest);
    // Only use supported lifecycle operations; documentation records have no session dependency.
    let cleanup = session.abort().and_then(|_| session.gc(false)).map(|_| ());
    let evidence = result?;
    cleanup?;
    if git(&repo, &["rev-parse", "--verify", "HEAD^{commit}"])? != revision {
        return Err(ClewError::new(
            ErrorCode::InputMutated,
            "service revision changed during documentation extraction",
        ));
    }
    let _lock = repository.lock()?;
    repository.atomic(&cache_path, &bytes(&evidence)?)?;
    Ok(evidence)
}

fn capture_session(
    session: &SessionAuthority,
    service: &Service,
    service_digest: &str,
) -> Result<ServiceEvidence, ClewError> {
    let ready = generation_service::ensure_session_generation(session)?;
    let state = StateAuthority::process_default()?;
    let store = CasStore::open(&state)?;
    let snapshot = generation_service::load_snapshot(&store, &ready)?;
    let contents: BTreeMap<_, _> = snapshot
        .worktree
        .iter()
        .filter_map(|e| e.content.as_ref().map(|c| (e.path.clone(), c.clone())))
        .collect();
    let mut facts = Vec::new();
    let mut count = 0usize;
    let mut total = 0u64;
    for compilation in &ready.compilations {
        let lease = store.read(&compilation.generation, MAX_EVIDENCE as usize)?;
        let generation: GenerationManifest =
            serde_json::from_slice(lease.bytes()).map_err(io_error)?;
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
            let value: Value = serde_json::from_slice(lease.bytes()).map_err(io_error)?;
            facts.push((value, fact.payload.digest.clone()));
            Ok(())
        })?;
    }
    if service.language == "kotlin" {
        facts = super::kotlin::project_facts(facts)?;
    }
    let mut files = BTreeMap::new();
    let wanted: BTreeSet<_> = facts
        .iter()
        .filter_map(|(f, _)| f["file"].as_str().map(str::to_owned))
        .chain(service.contract_files.iter().cloned())
        .collect();
    let mut source_bytes = 0usize;
    for file in wanted {
        if let Some(reference) = contents.get(&file) {
            let lease = store.read(reference, store::MAX_RECORD as usize)?;
            source_bytes = source_bytes.saturating_add(lease.bytes().len());
            if source_bytes > MAX_EVIDENCE as usize {
                return Err(ClewError::new(
                    ErrorCode::SliceBudgetExceeded,
                    "documentation source byte budget exceeded",
                ));
            }
            files.insert(
                file,
                String::from_utf8(lease.bytes().to_vec()).map_err(io_error)?,
            );
        }
    }
    project(
        service,
        &session.base_revision,
        service_digest,
        &format!("{:?}", session.runtime_mode).to_uppercase(),
        &ready.coverage,
        facts,
        &files,
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

fn add_source(
    evidence: &mut ServiceEvidence,
    service: &Service,
    files: &BTreeMap<String, String>,
    fact: &Value,
    binding: &str,
    identity: &str,
) -> Result<Option<String>, ClewError> {
    let Some(file) = fact["file"].as_str() else {
        return Ok(None);
    };
    let Some(text) = files.get(file) else {
        return Ok(None);
    };
    let (Some(start), Some(end)) = (fact["startLine"].as_u64(), fact["endLine"].as_u64()) else {
        return Ok(None);
    };
    let lines: Vec<_> = text.lines().collect();
    if start == 0 || end < start || end > lines.len() as u64 {
        return Ok(None);
    }
    let exact = lines[start as usize - 1..end as usize].join("\n");
    let id = source_id(&service.id, identity)?;
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
            authority: "EXACT_SNAPSHOT_TEXT".into(),
            url: source_link(service, &evidence.revision, file, start, end),
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

pub fn project(
    service: &Service,
    revision: &str,
    service_digest: &str,
    runtime_mode: &str,
    coverage: &str,
    facts: Vec<(Value, String)>,
    files: &BTreeMap<String, String>,
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
        let id = dependency_id(&service.id, "symbol", symbol)?;
        let source = add_source(&mut evidence, service, files, fact, binding, symbol)?;
        let mut normalized = strip_coordinates(fact);
        if let Some(s) = source.as_ref().and_then(|id| evidence.sources.get(id)) {
            normalized["sourceTokens"] = json!(java_tokens(&s.text));
        }
        let source_ids: Vec<_> = source.into_iter().collect();
        let observation = Observation {
            id: id.clone(),
            kind: "SYMBOL".into(),
            service: service.id.clone(),
            symbol: symbol.into(),
            digest: digest(&normalized)?,
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
                let identity = format!("{symbol}/event/{index}");
                let event_id = dependency_id(&service.id, "flow", &identity)?;
                let event_source =
                    add_source(&mut evidence, service, files, event, binding, &identity)?;
                let mut normalized = strip_coordinates(event);
                normalized["ordinal"] = json!(index);
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
        if let Some(spring) = fact.get("spring") {
            let metadata = spring_entrypoints::validate_metadata(
                spring,
                if service.language == "kotlin" {
                    "K2_RESOLVED_ANNOTATIONS"
                } else {
                    "JAVAC_RESOLVED_ANNOTATIONS"
                },
            )?;
            for (ordinal, entry) in metadata.entries.iter().enumerate() {
                let target = entry.target_symbol.as_deref().unwrap_or(symbol);
                let eid = source_id(&service.id, &format!("entrypoint/{target}/{ordinal}"))?;
                let trigger = spring_entrypoints::describe_trigger(entry);
                let route_id =
                    dependency_id(&service.id, "entrypoint", &format!("{target}/{ordinal}"))?;
                let normalized =
                    json!({"trigger":trigger,"binding":entry,"boundaries":metadata.boundaries});
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
    for path in &service.contract_files {
        let Some(text) = files.get(path) else {
            evidence
                .boundaries
                .push(format!("CONTRACT_SOURCE_UNAVAILABLE:{path}"));
            continue;
        };
        let value: Value = serde_yaml_ng::from_str(text)
            .map_err(|_| invalid("declared contract is not JSON/YAML"))?;
        if !value["openapi"]
            .as_str()
            .is_some_and(|v| v.starts_with("3.0."))
        {
            evidence
                .boundaries
                .push(format!("UNSUPPORTED_CONTRACT_VERSION:{path}"));
            continue;
        }
        let id = dependency_id(&service.id, "contract", path)?;
        let fact = json!({"file":path,"startLine":1,"endLine":text.lines().count()});
        let source = add_source(
            &mut evidence,
            service,
            files,
            &fact,
            &canonical::hash_bytes(text.as_bytes()),
            &format!("contract/{path}"),
        )?;
        if let Some(s) = source.as_ref().and_then(|s| evidence.sources.get_mut(s)) {
            s.authority = "DECLARED_OPENAPI".into();
        }
        evidence.observations.insert(
            id.clone(),
            Observation {
                id,
                kind: "CONTRACT".into(),
                service: service.id.clone(),
                symbol: path.clone(),
                digest: digest(&value)?,
                normalized: value.clone(),
                source_ids: source.into_iter().collect(),
            },
        );
        evidence.contracts.insert(path.clone(), value);
    }
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
        || e.extractor != EXTRACTOR
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
}
