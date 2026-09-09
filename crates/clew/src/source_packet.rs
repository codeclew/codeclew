//! Deterministic source selection for a client to attach before its first model request.
//! This is bounded source evidence, not a proof that a question has been answered.
use crate::canonical;
use crate::cas::{CasObject, CasStore};
use crate::error::{ClewError, ErrorCode};
use crate::generation_service::{load_session_generation, load_snapshot};
use crate::generation_v2::GenerationManifest;
use crate::repository_snapshot::{RepositoryInputSnapshot, WorktreeKind};
use crate::session::{SessionAuthority, SessionLanguage};
use crate::state::StateAuthority;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path};

pub const SCHEMA: &str = "codeclew-source-packet/1.0";
pub const MAX_STDOUT_BYTES: usize = 128 * 1024;
const MAX_FACTS: usize = 131_072;
const MAX_FACT_BYTES: u64 = 64 * 1024 * 1024;
const MAX_SOURCE_FILE_BYTES: usize = 2 * 1024 * 1024;
const MAX_SOURCE_READ_BYTES: usize = 16 * 1024 * 1024;
const MAX_SOURCE_BYTES: usize = 64 * 1024;
const MAX_DECLARATION_BYTES: usize = 32 * 1024;
const COMPLETE_FILE_BYTES: usize = 8 * 1024;
const MAX_TEST_BYTES: usize = 16 * 1024;
const MAX_TEST_FILES: usize = 4;
const MAX_DECLARATIONS: usize = 24;
const MAX_GRAPH_NODES: usize = 128;
const MAX_HOPS: usize = 2;

#[derive(Debug, Clone)]
struct Declaration {
    compilation: String,
    identity: String,
    identifiers: BTreeSet<String>,
    owner: Option<String>,
    class: bool,
    file: String,
    start: usize,
    end: usize,
    binding: String,
    calls: BTreeSet<String>,
    boundaries: BTreeSet<String>,
}

impl Declaration {
    fn source_key(&self) -> (&str, &str, usize, usize) {
        (&self.identity, &self.file, self.start, self.end)
    }
}

pub fn validate_identifiers(identifiers: &[String]) -> Result<(), ClewError> {
    if identifiers.is_empty()
        || identifiers.len() > 3
        || identifiers.iter().collect::<BTreeSet<_>>().len() != identifiers.len()
        || identifiers
            .iter()
            .any(|name| name.is_empty() || name.len() > 1024 || name.chars().any(char::is_control))
    {
        return Err(invalid(
            "source packet requires one to three distinct exact task identifiers",
        ));
    }
    Ok(())
}

pub fn create(session: &SessionAuthority, identifiers: &[String]) -> Result<Value, ClewError> {
    validate_identifiers(identifiers)?;
    session.require_open()?;
    if session.language != SessionLanguage::Kotlin {
        return Err(ClewError::new(
            ErrorCode::UnsupportedLanguage,
            "source packet call selection currently requires retained Kotlin K2 generations",
        ));
    }
    let state = StateAuthority::process_default()?;
    let store = CasStore::open(&state)?;
    let ready = load_session_generation(session)?;
    let snapshot = load_snapshot(&store, &ready)?;
    let mut declarations = Vec::new();
    let mut analysis_boundaries = BTreeSet::new();
    let mut visited = 0usize;
    let mut payload_bytes = 0u64;
    for compilation in &ready.compilations {
        let lease = store.read(&compilation.generation, MAX_FACT_BYTES as usize)?;
        let generation: GenerationManifest =
            serde_json::from_slice(lease.bytes()).map_err(internal)?;
        let mut facts = Vec::new();
        generation.visit_facts(&store, |fact| {
            visited += 1;
            if visited > MAX_FACTS {
                return Err(budget("source packet visited fact budget exceeded"));
            }
            // Local CFG and mutation facts are not inputs to this call-source projection.
            if fact.domain_uri.as_str() != crate::kotlin_adapter_v2::KOTLIN_FACTS_CAPABILITY
                || ![
                    "kotlin:metadata:",
                    "kotlin:descriptor:",
                    "kotlin:file:",
                    "kotlin:descriptor-boundary:",
                    "kotlin:relation-boundary:",
                ]
                .iter()
                .any(|prefix| fact.fact_key.starts_with(prefix))
            {
                return Ok(());
            }
            payload_bytes = payload_bytes
                .checked_add(fact.payload.size)
                .filter(|bytes| *bytes <= MAX_FACT_BYTES)
                .ok_or_else(|| budget("source packet fact payload budget exceeded"))?;
            let lease = store.read(&fact.payload, MAX_FACT_BYTES as usize)?;
            facts.push((
                serde_json::from_slice(lease.bytes()).map_err(internal)?,
                fact.payload.digest.clone(),
            ));
            Ok(())
        })?;
        let projected = crate::documentation::kotlin::project_facts(facts)?;
        analysis_boundaries.extend(
            projected
                .iter()
                .filter(|(fact, _)| fact["kind"] == "BOUNDARY")
                .filter_map(|(fact, _)| fact["code"].as_str().map(str::to_owned)),
        );
        declarations.extend(project_declarations(&compilation.compilation, projected)?);
    }
    declarations.sort_by(|a, b| {
        (&a.file, a.start, &a.identity, &a.compilation).cmp(&(
            &b.file,
            b.start,
            &b.identity,
            &b.compilation,
        ))
    });
    let mut packet = assemble(&store, &snapshot, &declarations, identifiers)?;
    packet["sessionId"] = json!(session.session_id);
    packet["baseRevision"] = json!(session.base_revision);
    packet["snapshotId"] = json!(snapshot.snapshot_id);
    packet["repositorySnapshot"] = json!(ready.repository_snapshot);
    packet["generationAuthority"] = json!({
        "coverage":ready.coverage, "certainty":ready.certainty,
        "obligations":ready.obligations,
        "compilations":ready.compilations.iter().map(|c| json!({
            "compilation":c.compilation, "generation":c.generation,
            "compilerVersion":c.compiler_version, "completeness":c.completeness,
        })).collect::<Vec<_>>(),
    });
    packet["analysisBoundaries"] = json!(analysis_boundaries);
    packet["preparation"] = json!({
        "modelCalls":0, "modelTokens":0, "visitedFacts":visited, "factPayloadBytes":payload_bytes,
    });
    packet["evidenceDigest"] = json!(evidence_digest(&packet)?);
    validate_stdout(&packet)?;
    Ok(packet)
}

fn project_declarations(
    compilation: &str,
    facts: Vec<(Value, String)>,
) -> Result<Vec<Declaration>, ClewError> {
    facts
        .into_iter()
        .filter(|(fact, _)| fact["kind"] == "DECLARATION")
        .map(|(fact, binding)| {
            let object = fact
                .as_object()
                .ok_or_else(|| invalid("packet declaration is not an object"))?;
            let mut identity_payload = object.clone();
            // The documentation projection gives constructors a display name.
            // It must not become another exact class-name declaration selector.
            if fact["declarationKind"] == "CONSTRUCTOR" {
                identity_payload.remove("name");
            }
            let file = string(&fact, "file")?.to_owned();
            if !safe_path(&file) {
                return Err(invalid("packet declaration source path is invalid"));
            }
            let start = offset(&fact, "start")?;
            let end = offset(&fact, "end")?;
            if start >= end {
                return Err(invalid("packet declaration range is empty or reversed"));
            }
            let mut boundaries = fact
                .pointer("/documentation/boundaries")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect::<BTreeSet<_>>();
            let mut calls = BTreeSet::new();
            if let Some(events) = fact
                .pointer("/documentation/events")
                .and_then(Value::as_array)
            {
                for event in events {
                    if matches!(event["kind"].as_str(), Some("CALL" | "CONSTRUCT")) {
                        if event["resolution"] == "COMPILER_EXACT" {
                            if let Some(target) = event["target"].as_str() {
                                calls.insert(target.into());
                            }
                        } else {
                            boundaries.insert("DOCUMENTATION_CALL_TARGET_UNRESOLVED".into());
                        }
                    }
                }
            } else if fact["declarationKind"] == "FUNCTION" {
                boundaries.insert("DECLARATION_CALL_FLOW_UNAVAILABLE".into());
            }
            Ok(Declaration {
                compilation: compilation.into(),
                identity: string(&fact, "symbolIdentity")?.into(),
                identifiers: crate::query_v2::declaration_identifiers(&identity_payload)
                    .into_iter()
                    .map(str::to_owned)
                    .collect(),
                owner: fact["ownerIdentity"].as_str().map(str::to_owned),
                class: fact["declarationKind"] == "CLASS",
                file,
                start,
                end,
                binding,
                calls,
                boundaries,
            })
        })
        .collect()
}

fn source_index(snapshot: &RepositoryInputSnapshot) -> BTreeMap<String, CasObject> {
    let mut files = snapshot
        .index
        .iter()
        .filter(|entry| entry.stage == 0)
        .map(|entry| (entry.path.clone(), entry.content.clone()))
        .collect::<BTreeMap<_, _>>();
    for entry in &snapshot.worktree {
        match (entry.kind, &entry.content) {
            (WorktreeKind::Regular, Some(content)) => {
                files.insert(entry.path.clone(), content.clone());
            }
            _ => {
                files.remove(&entry.path);
            }
        }
    }
    files
}

struct Sources<'a> {
    store: &'a CasStore,
    files: BTreeMap<String, CasObject>,
    loaded: BTreeMap<String, String>,
    windows: Vec<Value>,
    boundaries: BTreeMap<String, usize>,
    bytes: usize,
    read_bytes: usize,
    declarations: usize,
}

impl Sources<'_> {
    fn boundary(&mut self, code: &str) {
        *self.boundaries.entry(code.into()).or_default() += 1;
    }

    fn load(&mut self, file: &str) -> Result<Option<(CasObject, String)>, ClewError> {
        let Some(content) = self.files.get(file).cloned() else {
            self.boundary("SOURCE_OUTSIDE_RETAINED_SNAPSHOT");
            return Ok(None);
        };
        if content.size > MAX_SOURCE_FILE_BYTES as u64 {
            self.boundary("SOURCE_FILE_BYTE_BUDGET");
            return Ok(None);
        }
        if !self.loaded.contains_key(file) {
            if self.read_bytes.saturating_add(content.size as usize) > MAX_SOURCE_READ_BYTES {
                self.boundary("SOURCE_READ_BYTE_BUDGET");
                return Ok(None);
            }
            self.read_bytes += content.size as usize;
            let lease = self.store.read(&content, MAX_SOURCE_FILE_BYTES)?;
            let Ok(source) = std::str::from_utf8(lease.bytes()) else {
                self.boundary("SOURCE_NOT_UTF8");
                return Ok(None);
            };
            self.loaded.insert(file.into(), source.into());
        }
        Ok(Some((content, self.loaded[file].clone())))
    }

    fn declaration(&mut self, declaration: &Declaration, role: &str) -> Result<bool, ClewError> {
        let Some((content, source)) = self.load(&declaration.file)? else {
            return Ok(false);
        };
        if declaration.end > source.len()
            || !source.is_char_boundary(declaration.start)
            || !source.is_char_boundary(declaration.end)
        {
            return Err(invalid(
                "compiler declaration range does not bind retained UTF-8 source",
            ));
        }
        let (start, end) = if source.len() <= COMPLETE_FILE_BYTES {
            (0, source.len())
        } else {
            (
                source[..declaration.start].rfind('\n').map_or(0, |i| i + 1),
                source[declaration.end..]
                    .find('\n')
                    .map_or(source.len(), |i| declaration.end + i),
            )
        };
        let selection = json!({"role":role, "authority":"COMPILER_DECLARATION", "identity":declaration.identity,
            "compilation":declaration.compilation, "factBindingDigest":declaration.binding});
        if let Some(window) = self.windows.iter_mut().find(|w| {
            w["file"] == declaration.file
                && w["byteStart"].as_u64().is_some_and(|s| s <= start as u64)
                && w["byteEnd"].as_u64().is_some_and(|e| e >= end as u64)
        }) {
            let selections = window["selections"]
                .as_array_mut()
                .expect("source selections are an array");
            if !selections.contains(&selection) {
                selections.push(selection);
            }
            return Ok(true);
        }
        if end - start > MAX_DECLARATION_BYTES {
            self.boundary("COMPLETE_DECLARATION_BYTE_BUDGET");
            return Ok(false);
        }
        if self.declarations >= MAX_DECLARATIONS || self.bytes + end - start > MAX_SOURCE_BYTES {
            self.boundary("SOURCE_PACKET_BUDGET");
            return Ok(false);
        }
        self.add_window(&declaration.file, content, &source, start, end, selection);
        self.declarations += 1;
        Ok(true)
    }

    fn add_window(
        &mut self,
        file: &str,
        content: CasObject,
        source: &str,
        start: usize,
        end: usize,
        selection: Value,
    ) {
        let text = &source[start..end];
        let start_line = source[..start].bytes().filter(|b| *b == b'\n').count() + 1;
        self.bytes += text.len();
        self.windows.push(json!({"file":file, "contentRef":content, "byteStart":start, "byteEnd":end,
            "startLine":start_line, "endLine":start_line + text.lines().count().max(1) - 1,
            "text":text, "completeFile":start == 0 && end == source.len(), "selections":[selection]}));
    }

    fn tests(&mut self, roots: &[&Declaration]) -> Result<(), ClewError> {
        let stems = roots
            .iter()
            .filter_map(|d| Path::new(&d.file).file_stem()?.to_str())
            .collect::<BTreeSet<_>>();
        let names = stems
            .iter()
            .flat_map(|name| [format!("{name}Test.kt"), format!("{name}Tests.kt")])
            .collect::<BTreeSet<_>>();
        let candidates = self
            .files
            .keys()
            .filter(|file| {
                file.split('/')
                    .any(|part| part == "test" || part == "tests")
                    && Path::new(file)
                        .file_name()
                        .and_then(|s| s.to_str())
                        .is_some_and(|name| names.contains(name))
            })
            .cloned()
            .collect::<Vec<_>>();
        if candidates.is_empty() {
            self.boundary("NO_NAMED_TEST_COMPANION");
        }
        let mut bytes = 0usize;
        for (index, file) in candidates.iter().enumerate() {
            if index >= MAX_TEST_FILES {
                self.boundary("TEST_COMPANION_COUNT_BUDGET");
                continue;
            }
            let Some((content, source)) = self.load(file)? else {
                continue;
            };
            if bytes + source.len() > MAX_TEST_BYTES || self.bytes + source.len() > MAX_SOURCE_BYTES
            {
                self.boundary("COMPLETE_TEST_FILE_BYTE_BUDGET");
                continue;
            }
            if self
                .windows
                .iter()
                .any(|w| w["file"] == *file && w["completeFile"] == true)
            {
                continue;
            }
            bytes += source.len();
            self.add_window(
                file,
                content,
                &source,
                0,
                source.len(),
                json!({
                    "role":"TEST_CANDIDATE", "authority":"LEXICAL_TEST_COMPANION",
                    "relationVerified":false, "execution":"NOT_RUN",
                }),
            );
        }
        Ok(())
    }
}

fn assemble(
    store: &CasStore,
    snapshot: &RepositoryInputSnapshot,
    declarations: &[Declaration],
    identifiers: &[String],
) -> Result<Value, ClewError> {
    let mut roots = Vec::new();
    let mut root_indices = BTreeSet::new();
    let mut unique = true;
    for identifier in identifiers {
        let candidates = declarations
            .iter()
            .enumerate()
            .filter(|(_, d)| d.identifiers.contains(identifier))
            .collect::<Vec<_>>();
        let source_keys = candidates
            .iter()
            .map(|(_, d)| d.source_key())
            .collect::<BTreeSet<_>>();
        let status = match source_keys.len() {
            0 => "NOT_FOUND",
            1 => "UNIQUE",
            _ => "AMBIGUOUS",
        };
        unique &= status == "UNIQUE";
        roots.push(
            json!({"identifier":identifier, "status":status, "candidateCount":source_keys.len()}),
        );
        if status == "UNIQUE" {
            root_indices.extend(candidates.iter().map(|(index, _)| *index));
        }
    }
    let mut sources = Sources {
        store,
        files: source_index(snapshot),
        loaded: BTreeMap::new(),
        windows: Vec::new(),
        boundaries: BTreeMap::new(),
        bytes: 0,
        read_bytes: 0,
        declarations: 0,
    };
    let mut selected = BTreeSet::new();
    let mut root_sources_complete = unique;
    if unique {
        for index in &root_indices {
            if sources.declaration(&declarations[*index], "ROOT")? {
                selected.insert(*index);
            } else {
                root_sources_complete = false;
            }
        }
        let root_declarations = root_indices
            .iter()
            .map(|index| &declarations[*index])
            .collect::<Vec<_>>();
        sources.tests(&root_declarations)?;
        // A complete selected class already contains its own members. Their exact call
        // identities seed traversal without appending the same method source repeatedly.
        let retained_roots = selected.clone();
        for (index, d) in declarations.iter().enumerate() {
            if retained_roots
                .iter()
                .map(|i| &declarations[*i])
                .any(|root| {
                    root.class
                        && d.owner.as_deref() == Some(root.identity.as_str())
                        && d.compilation == root.compilation
                        && d.file == root.file
                        && d.start >= root.start
                        && d.end <= root.end
                })
            {
                if selected.len() == MAX_GRAPH_NODES {
                    sources.boundary("CLASS_MEMBER_GRAPH_BUDGET");
                    break;
                }
                selected.insert(index);
            }
        }
        let graph = CallGraph::new(declarations);
        let mut attempted = selected.clone();
        let mut frontier = selected.clone();
        for _ in 0..MAX_HOPS {
            let mut candidates = BTreeMap::<usize, &'static str>::new();
            for index in &frontier {
                for other in &graph.callers[*index] {
                    if !attempted.contains(other) {
                        candidates.insert(*other, "CALLER");
                    }
                }
                for other in &graph.callees[*index] {
                    if !attempted.contains(other) {
                        candidates.entry(*other).or_insert("CALLEE");
                    }
                }
            }
            let mut candidates = candidates.into_iter().collect::<Vec<_>>();
            candidates.sort_by_key(|(index, role)| (*role != "CALLER", *index));
            frontier.clear();
            for (index, role) in candidates {
                if attempted.len() >= MAX_GRAPH_NODES {
                    sources.boundary("CALL_GRAPH_NODE_BUDGET");
                    break;
                }
                attempted.insert(index);
                if sources.declaration(&declarations[index], role)? {
                    selected.insert(index);
                    frontier.insert(index);
                }
            }
            if frontier.is_empty() {
                break;
            }
        }
        for index in &selected {
            if graph.ambiguous.contains(index) {
                sources.boundary("AMBIGUOUS_CALL_TARGET_SOURCE");
            }
            if graph.external.contains(index) {
                sources.boundary("CALL_TARGET_OUTSIDE_SELECTED_COMPILATION");
            }
            if graph.callers[*index]
                .iter()
                .chain(&graph.callees[*index])
                .any(|other| !selected.contains(other))
            {
                sources.boundary("RELATED_SOURCE_NOT_INCLUDED");
            }
        }
    }
    for index in &selected {
        for boundary in &declarations[*index].boundaries {
            sources.boundary(boundary);
        }
    }
    let mut packet = json!({
        "schema":SCHEMA, "status":if !unique {"ABSTAIN"} else if !root_sources_complete {"ROOT_SOURCE_UNAVAILABLE"} else {"READY_WITH_LIMITS"},
        "roots":roots, "sources":sources.windows,
        "completeness":{"coverage":"PARTIAL", "questionSufficiency":"NOT_ESTABLISHED",
            "runtimeBehavior":"NOT_ESTABLISHED", "testExecution":"NOT_RUN"},
        "selectionPolicy":{"root":"EXACT_TASK_IDENTIFIER", "related":"RETAINED_K2_EXACT_CALL_TARGET",
            "directions":["CALLER", "CALLEE"], "maxHops":MAX_HOPS,
            "tests":"SNAPSHOT_FILENAME_COMPANION_UNVERIFIED", "wholeFileThresholdBytes":COMPLETE_FILE_BYTES,
            "maxSourceBytes":MAX_SOURCE_BYTES, "maxDeclarationBytes":MAX_DECLARATION_BYTES,
            "maxSourceDeclarations":MAX_DECLARATIONS, "maxGraphNodes":MAX_GRAPH_NODES,
            "propertiesAndTypeRelations":"NOT_FOLLOWED", "crossCompilationCalls":"NOT_FOLLOWED"},
        "boundaries":sources.boundaries,
        "counts":{"sourceBytes":sources.bytes, "sourceReadBytes":sources.read_bytes,
            "sourceWindows":0, "selectedGraphDeclarations":selected.len()},
    });
    packet["counts"]["sourceWindows"] = json!(packet["sources"].as_array().map_or(0, Vec::len));
    validate_stdout(&packet)?;
    Ok(packet)
}

struct CallGraph {
    callers: Vec<BTreeSet<usize>>,
    callees: Vec<BTreeSet<usize>>,
    ambiguous: BTreeSet<usize>,
    external: BTreeSet<usize>,
}

impl CallGraph {
    fn new(declarations: &[Declaration]) -> Self {
        let mut identities = BTreeMap::<(&str, &str), Vec<usize>>::new();
        for (index, declaration) in declarations.iter().enumerate() {
            identities
                .entry((&declaration.compilation, &declaration.identity))
                .or_default()
                .push(index);
        }
        let mut graph = Self {
            callers: vec![BTreeSet::new(); declarations.len()],
            callees: vec![BTreeSet::new(); declarations.len()],
            ambiguous: BTreeSet::new(),
            external: BTreeSet::new(),
        };
        for (index, declaration) in declarations.iter().enumerate() {
            for target in &declaration.calls {
                match identities
                    .get(&(declaration.compilation.as_str(), target.as_str()))
                    .map(Vec::as_slice)
                {
                    Some([other]) => {
                        graph.callees[index].insert(*other);
                        graph.callers[*other].insert(index);
                    }
                    Some(_) => {
                        graph.ambiguous.insert(index);
                    }
                    None => {
                        graph.external.insert(index);
                    }
                }
            }
        }
        graph
    }
}

pub fn evidence_digest(value: &Value) -> Result<String, ClewError> {
    let mut evidence = value.clone();
    evidence
        .as_object_mut()
        .ok_or_else(|| invalid("source packet must be an object"))?
        .remove("evidenceDigest");
    if let Some(preparation) = evidence
        .get_mut("preparation")
        .and_then(Value::as_object_mut)
    {
        preparation.remove("durationMs");
    }
    canonical::hash(&evidence).map_err(internal)
}

pub fn validate_stdout(value: &Value) -> Result<(), ClewError> {
    if canonical::bytes(value).map_err(internal)?.len() + 1 > MAX_STDOUT_BYTES {
        return Err(budget("source packet exceeds its 128 KiB stdout budget"));
    }
    Ok(())
}

fn safe_path(file: &str) -> bool {
    !file.is_empty()
        && !file.contains('\\')
        && !file.chars().any(char::is_control)
        && Path::new(file)
            .components()
            .all(|part| matches!(part, Component::Normal(_)))
}
fn string<'a>(value: &'a Value, key: &str) -> Result<&'a str, ClewError> {
    value[key]
        .as_str()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| invalid("packet declaration identity is unavailable"))
}
fn offset(value: &Value, key: &str) -> Result<usize, ClewError> {
    value[key]
        .as_u64()
        .and_then(|n| usize::try_from(n).ok())
        .ok_or_else(|| invalid("packet declaration byte range is unavailable"))
}
fn invalid(message: &str) -> ClewError {
    ClewError::new(ErrorCode::InvalidInput, message)
}
fn budget(message: &str) -> ClewError {
    ClewError::new(ErrorCode::SliceBudgetExceeded, message)
}
fn internal(error: impl std::fmt::Display) -> ClewError {
    ClewError::new(ErrorCode::Internal, error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repository_snapshot::{IndexEntry, WorktreeEntry};

    struct Fixture {
        _private: tempfile::TempDir,
        store: CasStore,
        snapshot: RepositoryInputSnapshot,
        declarations: Vec<Declaration>,
    }

    impl Fixture {
        fn new() -> Self {
            let private = tempfile::tempdir().unwrap();
            let state = StateAuthority::open(private.path().join("state")).unwrap();
            let store = CasStore::open(&state).unwrap();
            Self {
                _private: private,
                store,
                snapshot: RepositoryInputSnapshot {
                    schema: crate::repository_snapshot::SNAPSHOT_SCHEMA.into(),
                    snapshot_id: "snapshot:test".into(),
                    staged_view_digest: String::new(),
                    cached_view_digest: String::new(),
                    untracked_view_digest: String::new(),
                    index: vec![],
                    worktree: vec![],
                },
                declarations: vec![],
            }
        }

        fn file(&mut self, file: &str, source: &str) {
            let content = self
                .store
                .put("codeclew-repository-input-blob/2.0", source.as_bytes())
                .unwrap();
            self.snapshot.index.push(IndexEntry {
                path: file.into(),
                mode: 0o100644,
                stage: 0,
                git_oid: "0".repeat(40),
                content,
            });
        }

        fn function(&mut self, name: &str, calls: &[&str]) {
            let file = format!("src/main/kotlin/{name}.kt");
            let source = format!("fun {name}() {{ /* retained {name} body */ }}\n");
            self.file(&file, &source);
            self.declarations.push(Declaration {
                compilation: ":/main".into(),
                identity: name.into(),
                identifiers: BTreeSet::from([name.into()]),
                owner: None,
                class: false,
                file,
                start: 0,
                end: source.trim_end().len(),
                binding: "sha256:fact".into(),
                calls: calls.iter().map(|name| (*name).into()).collect(),
                boundaries: BTreeSet::new(),
            });
        }

        fn packet(&self, identifier: &str) -> Value {
            assemble(
                &self.store,
                &self.snapshot,
                &self.declarations,
                &[identifier.into()],
            )
            .unwrap()
        }
    }

    fn paths(packet: &Value) -> BTreeSet<&str> {
        packet["sources"]
            .as_array()
            .unwrap()
            .iter()
            .map(|source| source["file"].as_str().unwrap())
            .collect()
    }

    #[test]
    fn packet_follows_two_exact_call_hops_and_retains_unverified_test_source() {
        let mut f = Fixture::new();
        f.function("Root", &["Helper"]);
        f.function("Helper", &["Deep"]);
        f.function("Deep", &["Beyond"]);
        f.function("Beyond", &[]);
        f.function("Caller", &["Root"]);
        f.function("Producer", &["Caller"]);
        f.function("RootUnrelated", &[]);
        f.file(
            "src/test/kotlin/RootTest.kt",
            "// A snapshot test candidate, not an executed test.\n",
        );
        let packet = f.packet("Root");
        let selected = paths(&packet);
        for name in ["Root", "Helper", "Deep", "Caller", "Producer"] {
            assert!(selected.contains(format!("src/main/kotlin/{name}.kt").as_str()));
        }
        assert!(!selected.contains("src/main/kotlin/Beyond.kt"));
        assert!(!selected.contains("src/main/kotlin/RootUnrelated.kt"));
        assert!(
            packet["boundaries"]["RELATED_SOURCE_NOT_INCLUDED"]
                .as_u64()
                .unwrap()
                > 0
        );
        let test = packet["sources"]
            .as_array()
            .unwrap()
            .iter()
            .find(|s| s["file"] == "src/test/kotlin/RootTest.kt")
            .unwrap();
        assert_eq!(test["selections"][0]["authority"], "LEXICAL_TEST_COMPANION");
        assert_eq!(test["selections"][0]["relationVerified"], false);
        assert_eq!(test["selections"][0]["execution"], "NOT_RUN");
        assert_eq!(
            packet["completeness"]["questionSufficiency"],
            "NOT_ESTABLISHED"
        );
        assert_eq!(packet, f.packet("Root"));
    }

    #[test]
    fn ambiguous_roots_abstain_without_guessing_a_source() {
        let mut f = Fixture::new();
        f.function("first", &[]);
        f.function("second", &[]);
        f.declarations[0].identifiers.insert("overloaded".into());
        f.declarations[1].identifiers.insert("overloaded".into());
        let ambiguous = f.packet("overloaded");
        assert_eq!(ambiguous["status"], "ABSTAIN");
        assert_eq!(ambiguous["roots"][0]["candidateCount"], 2);
        assert!(paths(&ambiguous).is_empty());
        assert_eq!(f.packet("first")["status"], "READY_WITH_LIMITS");
    }

    #[test]
    fn call_selection_does_not_guess_overloads_or_cross_compilation_ownership() {
        let mut f = Fixture::new();
        f.function("Root", &["duplicate", "external"]);
        f.function("A", &[]);
        f.function("B", &[]);
        f.function("external", &[]);
        f.declarations[1].identity = "duplicate".into();
        f.declarations[2].identity = "duplicate".into();
        f.declarations[3].compilation = ":other/main".into();
        let packet = f.packet("Root");
        assert_eq!(paths(&packet).len(), 1);
        assert_eq!(packet["boundaries"]["AMBIGUOUS_CALL_TARGET_SOURCE"], 1);
        assert_eq!(
            packet["boundaries"]["CALL_TARGET_OUTSIDE_SELECTED_COMPILATION"],
            1
        );
    }

    #[test]
    fn complete_class_members_seed_calls_without_duplicate_method_source() {
        let mut f = Fixture::new();
        let file = "src/main/kotlin/Box.kt";
        let source = "class Box { fun act() = helper() }\n";
        f.file(file, source);
        let root = Declaration {
            compilation: ":/main".into(),
            identity: "class:Box".into(),
            identifiers: BTreeSet::from(["Box".into()]),
            owner: None,
            class: true,
            file: file.into(),
            start: 0,
            end: source.trim_end().len(),
            binding: "sha256:class".into(),
            calls: BTreeSet::new(),
            boundaries: BTreeSet::new(),
        };
        let mut member = root.clone();
        member.identity = "Box.act".into();
        member.class = false;
        member.owner = Some(root.identity.clone());
        member.identifiers = BTreeSet::from(["act".into()]);
        member.start = source.find("fun act").unwrap();
        member.end = source.find(" }").unwrap();
        member.calls.insert("helper".into());
        f.declarations.extend([root, member]);
        f.function("helper", &[]);
        let packet = f.packet("Box");
        assert_eq!(paths(&packet).len(), 2);
        assert!(paths(&packet).contains("src/main/kotlin/helper.kt"));
        assert_eq!(packet["counts"]["selectedGraphDeclarations"], 3);
    }

    #[test]
    fn deleted_or_symlinked_worktree_sources_do_not_resurrect_index_bytes() {
        for kind in [WorktreeKind::Missing, WorktreeKind::Symlink] {
            let mut f = Fixture::new();
            f.function("Root", &[]);
            f.snapshot.worktree.push(WorktreeEntry {
                path: f.declarations[0].file.clone(),
                kind,
                mode: 0,
                content: None,
            });
            let packet = f.packet("Root");
            assert_eq!(packet["status"], "ROOT_SOURCE_UNAVAILABLE");
            assert!(paths(&packet).is_empty());
            assert_eq!(packet["boundaries"]["SOURCE_OUTSIDE_RETAINED_SNAPSHOT"], 1);
        }
    }

    #[test]
    fn large_declarations_and_tests_are_omitted_whole_with_visible_limits() {
        let mut f = Fixture::new();
        let source = format!(
            "fun Root() {{ /* {} */ }}\n",
            "x".repeat(MAX_DECLARATION_BYTES)
        );
        f.file("src/main/kotlin/Root.kt", &source);
        f.function("other", &[]);
        f.declarations[0].file = "src/main/kotlin/Root.kt".into();
        f.declarations[0].identifiers = BTreeSet::from(["Root".into()]);
        f.declarations[0].end = source.trim_end().len();
        f.file(
            "src/test/kotlin/RootTest.kt",
            &"x".repeat(MAX_TEST_BYTES + 1),
        );
        let packet = f.packet("Root");
        assert!(paths(&packet).is_empty());
        assert_eq!(packet["status"], "ROOT_SOURCE_UNAVAILABLE");
        assert_eq!(packet["boundaries"]["COMPLETE_DECLARATION_BYTE_BUDGET"], 1);
        assert_eq!(packet["boundaries"]["COMPLETE_TEST_FILE_BYTE_BUDGET"], 1);
    }

    #[test]
    fn compiler_byte_ranges_must_match_utf8_boundaries() {
        let mut f = Fixture::new();
        f.function("Root", &[]);
        let source = "😀 fun Root() {}";
        let content = f
            .store
            .put("codeclew-repository-input-blob/2.0", source.as_bytes())
            .unwrap();
        f.snapshot.worktree.push(WorktreeEntry {
            path: f.declarations[0].file.clone(),
            kind: WorktreeKind::Regular,
            mode: 0o100644,
            content: Some(content),
        });
        f.declarations[0].start = 1;
        f.declarations[0].end = source.len();
        assert!(assemble(&f.store, &f.snapshot, &f.declarations, &["Root".into()]).is_err());
    }

    #[test]
    fn projection_follows_only_exact_documentation_targets_and_retains_boundaries() {
        let fact = json!({"kind":"DECLARATION", "declarationKind":"FUNCTION", "symbolIdentity":"callable:p/root#jvm:()V",
            "compilerCallableId":"p/root", "file":"Root.kt", "start":0, "end":10,
            "documentation":{"events":[
                {"kind":"CALL", "target":"exact", "resolution":"COMPILER_EXACT"},
                {"kind":"CALL", "target":"guessed", "resolution":"UNKNOWN"}],
                "boundaries":["CALLBACK_INVOCATION_ORDER_AND_COUNT_NOT_ESTABLISHED"]}});
        let projected =
            project_declarations(":/main", vec![(fact, "sha256:binding".into())]).unwrap();
        assert_eq!(projected[0].calls, BTreeSet::from(["exact".into()]));
        assert!(projected[0].identifiers.contains("root"));
        assert!(
            projected[0]
                .boundaries
                .contains("DOCUMENTATION_CALL_TARGET_UNRESOLVED")
        );
        assert!(
            projected[0]
                .boundaries
                .contains("CALLBACK_INVOCATION_ORDER_AND_COUNT_NOT_ESTABLISHED")
        );
    }

    #[test]
    fn evidence_digest_binds_source_and_admission_but_not_elapsed_time() {
        let mut value = json!({"sources":[{"text":"retained"}], "admission":{"status":"PASS"},
            "preparation":{"modelCalls":0, "durationMs":1}});
        let digest = evidence_digest(&value).unwrap();
        value["evidenceDigest"] = json!(digest);
        value["preparation"]["durationMs"] = json!(2);
        assert_eq!(evidence_digest(&value).unwrap(), digest);
        value["sources"][0]["text"] = json!("changed");
        assert_ne!(evidence_digest(&value).unwrap(), digest);
    }

    #[test]
    fn constructor_display_name_does_not_ambiguate_its_class_root() {
        let class = json!({"kind":"DECLARATION", "declarationKind":"CLASS", "symbolIdentity":"class:p/Box",
            "compilerClassId":"p/Box", "file":"Box.kt", "start":0, "end":10, "name":""});
        let constructor = json!({"kind":"DECLARATION", "declarationKind":"CONSTRUCTOR",
            "symbolIdentity":"constructor:p/Box.Box#jvm:()V", "compilerCallableId":"p/Box.Box",
            "file":"Box.kt", "start":0, "end":10, "name":"Box"});
        let declarations = project_declarations(
            ":/main",
            vec![
                (class, "class-binding".into()),
                (constructor, "constructor-binding".into()),
            ],
        )
        .unwrap();
        assert!(declarations[0].identifiers.contains("Box"));
        assert!(!declarations[1].identifiers.contains("Box"));
        assert!(
            declarations[1]
                .identifiers
                .contains("constructor:p/Box.Box#jvm:()V")
        );
    }
}
