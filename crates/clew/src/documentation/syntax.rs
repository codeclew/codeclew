//! Committed-source documentation. No project code, compiler, or build is executed.
use super::{analysis, digest, invalid, model::*, store};
use crate::{
    canonical,
    cas::CasStore,
    error::{ClewError, ErrorCode},
    repository_snapshot::{TrackedScopeLimits, capture_documentation_scope},
    state::StateAuthority,
};
use serde_json::{Value, json};
use std::{
    cell::Cell,
    collections::BTreeMap,
    path::Path,
    time::{Duration, Instant},
};
use tree_sitter::{Node, ParseOptions, Parser};

const MAX_FILE: usize = 2 * 1024 * 1024;
const MAX_NODES: usize = 200_000;
const MAX_FACTS: usize = 32_768;
fn budget() -> ClewError {
    ClewError::new(
        ErrorCode::SliceBudgetExceeded,
        "source documentation exceeds its bounded scope; narrow source roots",
    )
}

pub fn capture(service: &Service, repo: &Path) -> Result<ServiceEvidence, ClewError> {
    store::validate_service(service)?;
    let revision = analysis::git(
        repo,
        &[
            "rev-parse",
            "--verify",
            &format!("{}^{{commit}}", service.target_ref),
        ],
    )?;
    let state = StateAuthority::process_default()?;
    let cas = CasStore::open(&state)?;
    capture_with_store(service, repo, &revision, &cas)
}

fn capture_with_store(
    service: &Service,
    repo: &Path,
    revision: &str,
    cas: &CasStore,
) -> Result<ServiceEvidence, ClewError> {
    let config = service
        .source
        .as_ref()
        .ok_or_else(|| invalid("missing source scope"))?;
    let (snapshot, _) = capture_documentation_scope(
        repo,
        revision,
        &config.roots,
        cas,
        TrackedScopeLimits {
            max_files: 2048,
            max_file_bytes: MAX_FILE,
            max_total_bytes: 32 * 1024 * 1024,
            max_tree_entries: 100_000,
            max_tree_bytes: 16 * 1024 * 1024,
            max_tree_path_bytes: 4096,
        },
    )?;
    let mut e = ServiceEvidence {
        schema: "codeclew-documentation-service-evidence/1.0".into(),
        service: service.id.clone(),
        revision: revision.into(),
        service_digest: digest(service)?,
        extractor: SOURCE_EXTRACTOR.into(),
        runtime_mode: "COMMITTED_SOURCE_NO_BUILD".into(),
        coverage: "SYNTAX".into(),
        boundaries: vec![
            "CALL_TARGETS_UNRESOLVED".into(),
            "ORDER_LEXICAL_ONLY".into(),
            "DIALECT_DECLARED_NOT_COMPILER_VALIDATED".into(),
            "SCOPE_WATCH_CONSERVATIVE".into(),
        ],
        entrypoints: vec![],
        observations: BTreeMap::new(),
        sources: BTreeMap::new(),
        contracts: BTreeMap::new(),
    };
    let mut inventory = BTreeMap::new();
    let source_bytes = Cell::new(0usize);
    for entry in &snapshot.index {
        if !matches!(entry.mode, 0o100644 | 0o100755) {
            e.boundaries.push(format!("UNSAFE_FILE:{}", entry.path));
            inventory.insert(
                entry.path.clone(),
                json!({"mode":entry.mode,"blob":entry.git_oid,"coverage":"UNSAFE_FILE"}),
            );
            continue;
        }
        let lease = cas.read(&entry.content, MAX_FILE)?;
        let Ok(text) = std::str::from_utf8(lease.bytes()) else {
            inventory.insert(
                entry.path.clone(),
                json!({"digest":canonical::hash_bytes(lease.bytes()),"coverage":"FILE_ONLY"}),
            );
            e.boundaries.push(format!("NON_UTF8_FILE:{}", entry.path));
            continue;
        };
        let supported = match service.language.as_str() {
            "python" => entry.path.ends_with(".py"),
            "java" => entry.path.ends_with(".java"),
            "kotlin" => entry.path.ends_with(".kt") || entry.path.ends_with(".kts"),
            _ => false,
        };
        let file = File {
            service,
            path: &entry.path,
            text,
            snapshot: &snapshot.snapshot_id,
            blob: &entry.git_oid,
            source_bytes: &source_bytes,
        };
        if supported {
            let tree = parse(&service.language, text)?;
            let fingerprint = fingerprint(tree.root_node(), text)?;
            inventory.insert(entry.path.clone(), json!({"digest":fingerprint,"coverage":if tree.root_node().has_error(){"PARTIAL"}else{"SYNTAX"}}));
            extract(&file, tree.root_node(), &mut e)?;
        } else {
            inventory.insert(
                entry.path.clone(),
                json!({"digest":canonical::hash_bytes(text.as_bytes()),"coverage":"FILE_ONLY"}),
            );
            if !text.is_empty() {
                file.source(&mut e, &format!("file:{}", entry.path), 0, text.len())?;
            }
            e.boundaries.push(format!("FILE_ONLY:{}", entry.path));
        }
        if e.observations.len() > MAX_FACTS || e.sources.len() > MAX_FACTS {
            return Err(budget());
        }
    }
    if inventory.is_empty() {
        e.boundaries.push("EMPTY_SOURCE_SCOPE".into());
    }
    for root in &config.roots {
        if root != "."
            && !snapshot
                .index
                .iter()
                .any(|f| f.path == *root || f.path.starts_with(&format!("{root}/")))
        {
            e.boundaries.push(format!("MISSING_SOURCE_ROOT:{root}"));
        }
    }
    if e.boundaries.iter().any(|b| {
        b.starts_with("UNSAFE_FILE:")
            || b.starts_with("MISSING_SOURCE_ROOT:")
            || b == "EMPTY_SOURCE_SCOPE"
    }) {
        e.coverage = "PARTIAL".into();
    }
    let scope = json!({"roots":config.roots,"dialect":config.dialect,"language":service.language,"extractor":SOURCE_EXTRACTOR,"inventory":inventory,"boundaries":e.boundaries});
    observe(&mut e, "SOURCE_SCOPE", "source-scope", scope, vec![])?;
    e.boundaries.sort();
    e.boundaries.dedup();
    analysis::verify_evidence(&e)?;
    Ok(e)
}

fn parse(language: &str, text: &str) -> Result<tree_sitter::Tree, ClewError> {
    let mut parser = Parser::new();
    let grammar = match language {
        "python" => tree_sitter_python::LANGUAGE.into(),
        "java" => tree_sitter_java::LANGUAGE.into(),
        "kotlin" => tree_sitter_kotlin_ng::LANGUAGE.into(),
        _ => return Err(invalid("unsupported source language")),
    };
    parser
        .set_language(&grammar)
        .map_err(|_| invalid("pinned source grammar is incompatible"))?;
    let started = Instant::now();
    let mut progress = |_: &tree_sitter::ParseState| started.elapsed() > Duration::from_secs(2);
    parser
        .parse_with_options(
            &mut |offset, _| &text.as_bytes()[offset..],
            None,
            Some(ParseOptions::new().progress_callback(&mut progress)),
        )
        .ok_or_else(budget)
}

/// Topology and leaf spelling retain Python nesting, comments, strings, and numbers;
/// coordinates and inter-token whitespace do not make relocation a semantic edit.
fn fingerprint(root: Node<'_>, text: &str) -> Result<String, ClewError> {
    let mut tokens = Vec::new();
    let mut stack = vec![(root, 0usize)];
    while let Some((node, depth)) = stack.pop() {
        if tokens.len() >= MAX_NODES || depth > 256 {
            return Err(budget());
        }
        tokens.push((
            depth,
            node.kind(),
            if node.child_count() == 0 {
                node.utf8_text(text.as_bytes()).unwrap_or("")
            } else {
                ""
            },
        ));
        for i in (0..node.child_count()).rev() {
            stack.push((node.child(i).unwrap(), depth + 1));
        }
    }
    digest(&tokens)
}
fn children(node: Node<'_>) -> Vec<Node<'_>> {
    let mut cursor = node.walk();
    node.named_children(&mut cursor).collect()
}
fn spelling<'a>(node: Node<'_>, text: &'a str) -> &'a str {
    &text[node.byte_range()]
}
fn callable(kind: &str) -> bool {
    matches!(
        kind,
        "function_definition"
            | "function_declaration"
            | "method_declaration"
            | "constructor_declaration"
            | "secondary_constructor"
    )
}
fn class(kind: &str) -> bool {
    matches!(
        kind,
        "class_definition"
            | "class_declaration"
            | "interface_declaration"
            | "enum_declaration"
            | "record_declaration"
            | "object_declaration"
            | "companion_object"
    )
}
fn body(node: Node<'_>) -> Option<Node<'_>> {
    node.child_by_field_name("body").or_else(|| {
        children(node)
            .into_iter()
            .find(|n| matches!(n.kind(), "function_body" | "class_body" | "enum_class_body"))
    })
}

struct File<'a> {
    service: &'a Service,
    path: &'a str,
    text: &'a str,
    snapshot: &'a str,
    blob: &'a str,
    source_bytes: &'a Cell<usize>,
}
impl File<'_> {
    fn source(
        &self,
        e: &mut ServiceEvidence,
        identity: &str,
        start: usize,
        end: usize,
    ) -> Result<String, ClewError> {
        if e.sources.len() >= MAX_FACTS
            || end - start > MAX_FILE
            || self.source_bytes.get().saturating_add(end - start) > 32 * 1024 * 1024
        {
            return Err(budget());
        }
        self.source_bytes.set(self.source_bytes.get() + end - start);
        let exact = &self.text[start..end];
        if exact.is_empty() {
            return Err(invalid("empty source occurrence"));
        }
        let id = analysis::source_id(&self.service.id, identity)?;
        let start_line = self.text[..start].bytes().filter(|&b| b == b'\n').count() as u64 + 1;
        let end_line = start_line + exact.lines().count() as u64 - 1;
        e.sources.insert(
            id.clone(),
            Source {
                id: id.clone(),
                service: self.service.id.clone(),
                revision: e.revision.clone(),
                file: self.path.into(),
                start_line,
                end_line,
                text: exact.into(),
                text_digest: canonical::hash_bytes(exact.as_bytes()),
                evidence_digest: digest(&(SOURCE_EXTRACTOR, self.snapshot, self.blob, start, end))?,
                authority: "EXACT_SNAPSHOT_TEXT".into(),
                occurrence: Some(SourceOccurrence {
                    snapshot: self.snapshot.into(),
                    blob: self.blob.into(),
                    start_byte: start,
                    end_byte: end,
                }),
                url: analysis::source_link(
                    self.service,
                    &e.revision,
                    self.path,
                    start_line,
                    end_line,
                ),
            },
        );
        Ok(id)
    }
}
fn observe(
    e: &mut ServiceEvidence,
    kind: &str,
    symbol: &str,
    normalized: Value,
    source_ids: Vec<String>,
) -> Result<String, ClewError> {
    let id = analysis::dependency_id(&e.service, &kind.to_lowercase(), symbol)?;
    e.observations.insert(
        id.clone(),
        Observation {
            id: id.clone(),
            kind: kind.into(),
            service: e.service.clone(),
            symbol: symbol.into(),
            digest: digest(&normalized)?,
            normalized,
            source_ids,
        },
    );
    Ok(id)
}

fn extract(file: &File<'_>, root: Node<'_>, e: &mut ServiceEvidence) -> Result<(), ClewError> {
    if root.has_error() {
        e.coverage = "PARTIAL".into();
        e.boundaries.push(format!("PARSE_ERROR:{}", file.path));
    }
    let package = children(root)
        .into_iter()
        .find(|n| matches!(n.kind(), "package_declaration" | "package_header"))
        .and_then(|n| {
            children(n).into_iter().find(|n| {
                matches!(
                    n.kind(),
                    "identifier" | "scoped_identifier" | "qualified_identifier"
                )
            })
        })
        .map(|n| spelling(n, file.text).to_owned())
        .unwrap_or_else(|| {
            if file.service.language == "python" {
                file.path.trim_end_matches(".py").replace('/', ".")
            } else {
                String::new()
            }
        });
    let mut stack = vec![(root, vec![], 0usize)];
    let mut declarations = Vec::new();
    let mut details = Vec::new();
    let mut visited = 0;
    while let Some((node, owners, depth)) = stack.pop() {
        visited += 1;
        if visited > MAX_NODES || depth > 256 {
            return Err(budget());
        }
        let name = node
            .child_by_field_name("name")
            .map(|n| spelling(n, file.text).to_owned());
        let mut nested = owners.clone();
        if callable(node.kind()) || class(node.kind()) {
            if let Some(name) = name {
                let owner = if owners.is_empty() {
                    format!("package:{package}")
                } else {
                    format!("class:{}", owners.join("."))
                };
                let signature_end = body(node)
                    .map(|n| n.start_byte())
                    .unwrap_or(node.end_byte());
                // Header tokenization retains receiver, modifiers, defaults, and declared types.
                let header = &file.text[node.start_byte()..signature_end];
                let header_key = super::analysis::java_tokens(header);
                let identity = format!(
                    "source:{}/{owner}/{name}/{}",
                    file.path,
                    &digest(&header_key)?[7..27]
                );
                declarations.push((node, owner, name.clone(), identity));
                if nested.is_empty() && !package.is_empty() {
                    nested.push(package.clone());
                }
                nested.push(name);
            }
        } else if matches!(
            node.kind(),
            "import_declaration"
                | "import_header"
                | "import_statement"
                | "import_from_statement"
                | "field_declaration"
                | "property_declaration"
                | "assignment"
                | "annotation"
                | "marker_annotation"
        ) {
            let identity = format!(
                "syntax:{}/{}/{}/{}",
                file.path,
                owners.join("."),
                node.kind(),
                fingerprint(node, file.text)?
            );
            details.push((node, identity));
        }
        for child in children(node).into_iter().rev() {
            stack.push((child, nested.clone(), depth + 1));
        }
    }
    let mut detail_counts = BTreeMap::new();
    for (_, id) in &details {
        *detail_counts.entry(id.clone()).or_insert(0) += 1;
    }
    for (node, mut identity) in details {
        if detail_counts[&identity] > 1 {
            identity.push_str(&format!("/ambiguous:{}", node.start_byte()));
            e.boundaries.push("AMBIGUOUS_SOURCE_DETAIL".into());
        }
        let source = file.source(e, &identity, node.start_byte(), node.end_byte())?;
        observe(
            e,
            "SYNTAX_DETAIL",
            &identity,
            json!({"authority":"SYNTAX","syntaxKind":node.kind(),"text":spelling(node,file.text)}),
            vec![source],
        )?;
    }
    let mut counts = BTreeMap::new();
    for (_, _, _, id) in &declarations {
        *counts.entry(id.clone()).or_insert(0) += 1;
    }
    for (node, owner, name, mut identity) in declarations {
        if counts[&identity] > 1 {
            e.boundaries.push("AMBIGUOUS_SOURCE_DECLARATION".into());
            identity.push_str(&format!("/ambiguous:{}", node.start_byte()));
        }
        let source = file.source(e, &identity, node.start_byte(), node.end_byte())?;
        let mut events = Vec::new();
        if callable(node.kind()) {
            let mut stack = body(node)
                .into_iter()
                .map(|n| (n, None::<usize>, 0usize))
                .collect::<Vec<_>>();
            while let Some((n, parent, depth)) = stack.pop() {
                if depth > 256 || events.len() > MAX_FACTS {
                    return Err(budget());
                }
                let mut nested_parent = parent;
                if callable(n.kind()) || class(n.kind()) {
                    continue;
                }
                if let Some(kind) = event_kind(n) {
                    let index = events.len();
                    let event_identity = format!("{identity}/event/{index}");
                    let source = file.source(e, &event_identity, n.start_byte(), n.end_byte())?;
                    let event = json!({"kind":kind,"syntaxKind":n.kind(),"authority":"SYNTAX","ordering":"LEXICAL_ONLY","text":spelling(n,file.text),"fingerprint":fingerprint(n,file.text)?,"targetStatus":"UNRESOLVED","ordinal":index,"parentOrdinal":parent,"nestingDepth":depth});
                    observe(e, "FLOW", &event_identity, event.clone(), vec![source])?;
                    // FLOW lookup uses enclosing symbol, not its logical event identity.
                    let id = analysis::dependency_id(&e.service, "flow", &event_identity)?;
                    e.observations.get_mut(&id).unwrap().symbol = identity.clone();
                    events.push(event);
                    nested_parent = Some(index);
                }
                for child in children(n).into_iter().rev() {
                    stack.push((child, nested_parent, depth + 1));
                }
            }
        }
        let parameters = node.child_by_field_name("parameters").or_else(|| {
            children(node)
                .into_iter()
                .find(|n| n.kind() == "function_value_parameters")
        });
        let parameter_types: Vec<_> = parameters
            .into_iter()
            .flat_map(children)
            .map(|n| {
                n.child_by_field_name("type")
                    .map(|t| spelling(t, file.text).to_owned())
                    .unwrap_or_else(|| spelling(n, file.text).to_owned())
            })
            .collect();
        let normalized = json!({"kind":"DECLARATION","authority":"SYNTAX","name":name,"ownerIdentity":owner,"symbolIdentity":identity,"syntaxKind":node.kind(),"fingerprint":fingerprint(node,file.text)?,"documentation":{"parameterTypes":parameter_types,"events":events,"boundaries":["CALL_TARGETS_UNRESOLVED","ORDER_LEXICAL_ONLY"]}});
        let dep = observe(e, "SYMBOL", &identity, normalized, vec![source.clone()])?;
        if callable(node.kind()) {
            e.entrypoints.push(Entrypoint {
                id: analysis::source_id(&e.service, &format!("entry:{identity}"))?,
                service: e.service.clone(),
                symbol: identity,
                kind: "SOURCE_DECLARATION".into(),
                trigger: json!({"authority":"SYNTAX","name":name,"owner":owner}),
                source_ids: vec![source],
                dependency_ids: vec![dep],
                boundaries: vec![
                    "TRIGGER_NOT_FRAMEWORK_RESOLVED".into(),
                    "CALL_TARGETS_UNRESOLVED".into(),
                ],
            });
        }
    }
    Ok(())
}
fn event_kind(node: Node<'_>) -> Option<&'static str> {
    Some(match node.kind() {
        "if_statement"
        | "if_expression"
        | "conditional_expression"
        | "when_expression"
        | "switch_expression"
        | "switch_statement" => "IF",
        "for_statement" | "while_statement" | "do_while_statement" | "enhanced_for_statement" => {
            "LOOP"
        }
        "return_statement" | "return_expression" => "RETURN",
        "throw_statement" | "throw_expression" | "raise_statement" => "THROW",
        "try_statement" | "try_expression" => "TRY",
        "finally_clause" | "finally_block" => "FINALLY",
        "break_statement" => "BREAK",
        "continue_statement" => "CONTINUE",
        "lambda" | "lambda_literal" | "lambda_expression" => "DEFERRED",
        "call" | "call_expression" | "method_invocation" | "object_creation_expression" => "CALL",
        "navigation_expression" => "ACCESS",
        "binary_expression"
            if (0..node.child_count()).any(|i| node.child(i).is_some_and(|n| n.kind() == "?:")) =>
        {
            "IF"
        }
        "elvis_expression" => "IF",
        _ => return None,
    })
}

/// Attach compiler evidence only to unique, equal-revision source ranges. Syntax
/// roots and authored subjects remain stable; unmatched compiler facts stay gaps.
pub(super) fn enrich(
    source: &mut ServiceEvidence,
    semantic: Result<ServiceEvidence, ClewError>,
) -> Result<(), ClewError> {
    let mut attached = Vec::new();
    let mut unresolved = 0usize;
    let status = match semantic {
        Ok(semantic)
            if semantic.revision == source.revision && semantic.service == source.service =>
        {
            analysis::verify_evidence(&semantic)?;
            for fact in semantic
                .observations
                .values()
                .filter(|o| o.kind == "SYMBOL")
            {
                let candidates: Vec<_> = source
                    .observations
                    .values()
                    .filter(|o| {
                        o.kind == "SYMBOL"
                            && o.normalized["name"] == fact.normalized["name"]
                            && o.source_ids.iter().any(|id| {
                                fact.source_ids.iter().any(|other| {
                                    let a = &source.sources[id];
                                    let Some(b) = semantic.sources.get(other) else {
                                        return false;
                                    };
                                    a.file == b.file
                                        && a.start_line == b.start_line
                                        && a.end_line == b.end_line
                                })
                            })
                    })
                    .map(|o| (o.symbol.clone(), o.source_ids.clone()))
                    .collect();
                if candidates.len() != 1 {
                    unresolved += 1;
                    continue;
                }
                let (symbol, sources) = &candidates[0];
                // Keep the provider's own identity, normalized facts, and boundaries.
                let normalized = json!({"authority":"SEMANTIC","sourceSymbol":symbol,"semanticSymbol":fact.symbol,"producer":semantic.extractor,"runtimeMode":semantic.runtime_mode,"coverage":semantic.coverage,"boundaries":semantic.boundaries,"serviceDigest":semantic.service_digest,"fact":fact.normalized,"mapping":"UNIQUE_EQUAL_REVISION_FILE_AND_LINE_RANGE"});
                attached.push(observe(
                    source,
                    "SEMANTIC_SYMBOL",
                    symbol,
                    normalized,
                    sources.clone(),
                )?);
            }
            json!({"status":"AVAILABLE","producer":semantic.extractor,"runtimeMode":semantic.runtime_mode,"coverage":semantic.coverage,"boundaries":semantic.boundaries,"attached":attached,"unmapped":unresolved})
        }
        Ok(_) => json!({"status":"UNAVAILABLE","reason":"SEMANTIC_REVISION_MISMATCH"}),
        Err(error) => json!({"status":"UNAVAILABLE","reason":error.code}),
    };
    if status["status"] == "UNAVAILABLE" {
        source
            .boundaries
            .push("SEMANTIC_PROVIDER_UNAVAILABLE_SOURCE_REMAINS_READABLE".into());
    }
    if unresolved > 0 {
        source
            .boundaries
            .push("SEMANTIC_SOURCE_MAPPING_UNAVAILABLE".into());
    }
    // Include availability and all attached fact digests in the conservative scope.
    let overlay = json!({"provider":status,"facts":attached.iter().map(|id| (&source.observations[id].id,&source.observations[id].digest)).collect::<BTreeMap<_,_>>()});
    let scope = source
        .observations
        .values_mut()
        .find(|o| o.kind == "SOURCE_SCOPE")
        .ok_or_else(|| invalid("source scope observation missing"))?;
    scope.normalized["semantic"] = overlay;
    scope.digest = digest(&scope.normalized)?;
    source.boundaries.sort();
    source.boundaries.dedup();
    analysis::verify_evidence(source)
}

#[cfg(test)]
pub(super) fn source_for_provider_test(
    service: &Service,
    provider: &ServiceEvidence,
    files: &BTreeMap<String, String>,
) -> ServiceEvidence {
    let mut source = provider.clone();
    source.extractor = SOURCE_EXTRACTOR.into();
    source.runtime_mode = "COMMITTED_SOURCE_NO_BUILD".into();
    source.coverage = "SYNTAX".into();
    source.observations.clear();
    source.sources.clear();
    source.entrypoints.clear();
    source.boundaries.clear();
    source.contracts.clear();
    let bytes = Cell::new(0);
    for (path, text) in files {
        extract(
            &File {
                service,
                path,
                text,
                snapshot: "fixture-snapshot",
                blob: &canonical::hash_bytes(text.as_bytes()),
                source_bytes: &bytes,
            },
            parse(&service.language, text).unwrap().root_node(),
            &mut source,
        )
        .unwrap();
    }
    observe(
        &mut source,
        "SOURCE_SCOPE",
        "source-scope",
        json!({"fixtureFiles":digest(files).unwrap()}),
        vec![],
    )
    .unwrap();
    source
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, process::Command};
    const KOTLIN: &str = include_str!(
        "../../../../fixtures/durable-docs-source/kotlin/src/main/kotlin/example/Reservations.kt"
    );
    const JAVA: &str = include_str!(
        "../../../../fixtures/durable-docs-source/java/src/main/java/example/Reservations.java"
    );
    const PYTHON: &str = include_str!("../../../../fixtures/durable-docs-source/python/orders.py");
    fn service(language: &str) -> Service {
        serde_json::from_value(json!({"schema":"codeclew-documentation-service/1.0","id":"orders","title":"Orders","repositoryId":"orders","repository":"https://example.invalid/orders","language":language,"profile":"source-syntax","targetRef":"HEAD","source":{"roots":["."],"dialect":if language=="kotlin"{"1.9"}else{"declared"}}})).unwrap()
    }
    fn fixture(language: &str, text: &str) -> ServiceEvidence {
        let service = service(language);
        let tree = parse(language, text).unwrap();
        assert!(
            !tree.root_node().has_error(),
            "{}",
            tree.root_node().to_sexp()
        );
        let mut e = ServiceEvidence {
            schema: "codeclew-documentation-service-evidence/1.0".into(),
            service: service.id.clone(),
            revision: "1".repeat(40),
            service_digest: digest(&service).unwrap(),
            extractor: SOURCE_EXTRACTOR.into(),
            runtime_mode: "COMMITTED_SOURCE_NO_BUILD".into(),
            coverage: "SYNTAX".into(),
            boundaries: vec![],
            entrypoints: vec![],
            observations: BTreeMap::new(),
            sources: BTreeMap::new(),
            contracts: BTreeMap::new(),
        };
        extract(
            &File {
                service: &service,
                path: "source",
                text,
                snapshot: "snapshot",
                blob: "blob",
                source_bytes: &Cell::new(0),
            },
            tree.root_node(),
            &mut e,
        )
        .unwrap();
        analysis::verify_evidence(&e).unwrap();
        e
    }
    #[test]
    fn three_languages_expose_real_declarations_and_unresolved_flow_without_compilers() {
        for (language, text) in [("python", PYTHON), ("java", JAVA), ("kotlin", KOTLIN)] {
            let e = fixture(language, text);
            assert!(e.entrypoints.iter().any(|p| p.trigger["name"] == "reserve"));
            for kind in ["IF", "CALL", "RETURN"] {
                assert!(
                    e.observations
                        .values()
                        .any(|o| o.kind == "FLOW" && o.normalized["kind"] == kind),
                    "{language} lacks {kind}"
                );
            }
            assert!(
                e.observations
                    .values()
                    .filter(|o| o.kind == "FLOW")
                    .all(|o| o.normalized.get("target").is_none()
                        && o.normalized["authority"] == "SYNTAX")
            );
            for source in e.sources.values() {
                let range = source.occurrence.as_ref().unwrap();
                assert_eq!(source.text, text[range.start_byte..range.end_byte]);
            }
        }
        let e = fixture("kotlin", KOTLIN);
        for name in ["normalizedSku", "batch", "submit"] {
            assert!(e.entrypoints.iter().any(|p| p.trigger["name"] == name));
        }
        for kind in ["DEFERRED", "LOOP", "ACCESS"] {
            assert!(
                e.observations
                    .values()
                    .any(|o| o.kind == "FLOW" && o.normalized["kind"] == kind),
                "missing {kind}"
            );
        }
        assert!(e.sources.values().any(|s| s.text.contains("café")));
        assert!(
            e.observations
                .values()
                .any(|o| o.kind == "FLOW" && o.normalized["syntaxKind"] == "when_expression")
        );
        assert!(e.observations.values().any(|o| {
            o.kind == "FLOW"
                && o.normalized["kind"] == "IF"
                && o.normalized["text"]
                    .as_str()
                    .is_some_and(|text| text.contains("?:"))
        }));
        assert!(
            e.observations
                .values()
                .any(|o| o.kind == "FLOW" && o.normalized["parentOrdinal"].is_number())
        );
    }
    #[test]
    fn relocation_preserves_function_identity_but_literals_and_nesting_change_fingerprint() {
        for (language, text) in [("python", PYTHON), ("java", JAVA), ("kotlin", KOTLIN)] {
            let original = fixture(language, text);
            let relocated = fixture(language, &format!("\n\n{text}"));
            assert_eq!(original.entrypoints, relocated.entrypoints);
            let facts = |e: &ServiceEvidence| {
                e.observations
                    .values()
                    .filter(|o| matches!(o.kind.as_str(), "SYMBOL" | "FLOW"))
                    .map(|o| (o.id.clone(), o.digest.clone()))
                    .collect::<BTreeMap<_, _>>()
            };
            assert_eq!(facts(&original), facts(&relocated));
            assert_ne!(original.sources, relocated.sources);
            let tree = parse(language, text).unwrap();
            let modified = text.replace("café", "tea");
            let changed = parse(language, &modified).unwrap();
            assert_ne!(
                fingerprint(tree.root_node(), text).unwrap(),
                fingerprint(changed.root_node(), &modified).unwrap()
            );
        }
        let a = "def f(x):\n    if x:\n        return 1\n    return 2\n";
        let b = "def f(x):\n    if x:\n        return 1\n        return 2\n";
        assert_ne!(
            fingerprint(parse("python", a).unwrap().root_node(), a).unwrap(),
            fingerprint(parse("python", b).unwrap().root_node(), b).unwrap()
        );
    }
    fn git(repo: &Path, args: &[&str]) -> String {
        let output = Command::new("git")
            .current_dir(repo)
            .args(args)
            .env("GIT_AUTHOR_NAME", "Fixture")
            .env("GIT_AUTHOR_EMAIL", "fixture@example.invalid")
            .env("GIT_COMMITTER_NAME", "Fixture")
            .env("GIT_COMMITTER_EMAIL", "fixture@example.invalid")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap().trim().into()
    }
    #[test]
    fn committed_scope_ignores_dirty_text_and_watches_helpers_config_and_additions() {
        let repo = tempfile::tempdir().unwrap();
        let state = tempfile::tempdir().unwrap();
        let cas =
            CasStore::open(&StateAuthority::open(state.path().join("state")).unwrap()).unwrap();
        git(repo.path(), &["init", "-q"]);
        fs::write(repo.path().join("orders.py"), PYTHON).unwrap();
        fs::write(repo.path().join("policy.py"), "MAX_QUANTITY = 100\n").unwrap();
        git(repo.path(), &["add", "."]);
        git(repo.path(), &["commit", "-qm", "Initial"]);
        let revision = git(repo.path(), &["rev-parse", "HEAD"]);
        let service = service("python");
        let first = capture_with_store(&service, repo.path(), &revision, &cas).unwrap();
        fs::write(repo.path().join("orders.py"), "invalid dirty input").unwrap();
        assert_eq!(
            first,
            capture_with_store(&service, repo.path(), &revision, &cas).unwrap()
        );
        fs::write(repo.path().join("orders.py"), PYTHON).unwrap();
        let scope = |e: &ServiceEvidence| {
            e.observations
                .values()
                .find(|o| o.kind == "SOURCE_SCOPE")
                .unwrap()
                .digest
                .clone()
        };
        let mut previous = first;
        for (path, content) in [
            ("policy.py", "MAX_QUANTITY = 50\n"),
            ("settings.json", "{\"enabled\":false}"),
            ("new.py", "def extra():\n    return 1\n"),
        ] {
            fs::write(repo.path().join(path), content).unwrap();
            git(repo.path(), &["add", "."]);
            git(repo.path(), &["commit", "-qm", "Change scope"]);
            let next = capture_with_store(
                &service,
                repo.path(),
                &git(repo.path(), &["rev-parse", "HEAD"]),
                &cas,
            )
            .unwrap();
            assert_ne!(scope(&previous), scope(&next));
            previous = next;
        }
    }
    #[test]
    fn incomplete_syntax_retains_explicit_partial_evidence() {
        let text = "def good():\n    return 1\n\ndef bad(\n";
        let tree = parse("python", text).unwrap();
        assert!(tree.root_node().has_error());
        assert!(fingerprint(tree.root_node(), text).is_ok());
    }
    #[test]
    fn semantic_overlay_preserves_source_roots_and_failure_invalidates_its_dependencies() {
        let mut source = fixture("java", JAVA);
        observe(
            &mut source,
            "SOURCE_SCOPE",
            "source-scope",
            json!({"inventory":{}}),
            vec![],
        )
        .unwrap();
        let before = source.clone();
        let mut provider = source.clone();
        provider.extractor = EXTRACTOR.into();
        provider.observations.retain(|_, o| o.kind == "SYMBOL");
        for fact in provider.observations.values_mut() {
            fact.normalized["authority"] = json!("JAVAC");
            fact.digest = digest(&fact.normalized).unwrap();
        }
        enrich(&mut source, Ok(provider.clone())).unwrap();
        assert_eq!(source.entrypoints, before.entrypoints);
        assert_eq!(source.sources, before.sources);
        assert!(
            source
                .observations
                .values()
                .any(|o| o.kind == "SEMANTIC_SYMBOL")
        );
        let checked = super::super::check::assemble(
            "input".into(),
            BTreeMap::from([("orders".into(), source.clone())]),
            BTreeMap::new(),
            &BTreeMap::new(),
            &BTreeMap::new(),
        )
        .unwrap();
        let overlay = source
            .observations
            .values()
            .find(|o| o.kind == "SEMANTIC_SYMBOL")
            .unwrap();
        let bound = super::super::bindings::fragment(
            "service:orders",
            &"A semantic claim",
            std::slice::from_ref(&overlay.id),
            &overlay.source_ids,
            &checked,
        )
        .unwrap();
        let mut lost = before.clone();
        enrich(&mut lost, Err(invalid("provider unavailable"))).unwrap();
        assert_eq!(lost.entrypoints, before.entrypoints);
        assert!(!lost.observations.contains_key(&overlay.id));
        let scope = lost
            .observations
            .values()
            .find(|o| o.kind == "SOURCE_SCOPE")
            .unwrap();
        assert_ne!(bound.dependencies[&scope.id], scope.digest);
        provider.revision = "2".repeat(40);
        let mut wrong_revision = before;
        enrich(&mut wrong_revision, Ok(provider)).unwrap();
        assert!(
            !wrong_revision
                .observations
                .values()
                .any(|o| o.kind == "SEMANTIC_SYMBOL")
        );
    }

    #[test]
    fn duplicate_names_and_syntax_roots_do_not_create_resolved_handoffs() {
        let source = fixture(
            "python",
            "def duplicate(x):\n    return 1\ndef duplicate(x):\n    return 2\n",
        );
        assert_eq!(source.entrypoints.len(), 2);
        assert_ne!(source.entrypoints[0].id, source.entrypoints[1].id);
        let selected = Selector {
            language: "python".into(),
            owner: "package:source".into(),
            name: "duplicate".into(),
            parameter_types: None,
        };
        assert_eq!(analysis::resolve(Some(&selected), &source).len(), 2);
        let source = fixture("java", JAVA);
        let interaction:Interaction=serde_json::from_value(json!({"schema":"codeclew-documentation-interaction/1.0","id":"declared","title":"Declared handoff","from":{"service":"orders","selector":{"language":"java","owner":"example.Reservations","name":"reserve"}},"to":{"service":"orders","selector":{"language":"java","owner":"example.Reservations","name":"reserve"}},"transport":{"kind":"http","method":"POST","path":"/reserve"},"declaration":{"origin":"engineer","rationale":"Declared architecture; runtime linkage is not proven."}})).unwrap();
        let result = super::super::check::check_interaction(
            &interaction,
            &BTreeMap::from([("orders".into(), source)]),
        )
        .unwrap();
        assert_eq!(result.from.status, "SOURCE_MATCH");
        assert_eq!(result.to.status, "SOURCE_MATCH");
        assert_ne!(result.call_site.status, "RESOLVED");
    }
    #[test]
    fn partial_syntax_and_empty_catalogues_never_look_fresh() {
        use crate::documentation::{bindings, check, render};
        for (language, valid, broken) in [
            ("python", PYTHON, "def broken(\n"),
            ("java", JAVA, "class Broken { void broken(\n"),
            ("kotlin", KOTLIN, "fun broken(\n"),
        ] {
            let service = service(language);
            let provider = fixture(language, valid);
            let partial = source_for_provider_test(
                &service,
                &provider,
                &BTreeMap::from([("source".into(), broken.into())]),
            );
            assert_eq!(partial.coverage, "PARTIAL");
            let checked = check::assemble(
                "input".into(),
                BTreeMap::from([("orders".into(), partial)]),
                BTreeMap::new(),
                &BTreeMap::new(),
                &BTreeMap::new(),
            )
            .unwrap();
            let baseline = render::make_bindings(&checked, BTreeMap::new()).unwrap();
            assert!(
                baseline
                    .fragments
                    .contains_key("service:orders/source-scope")
            );
            assert_eq!(
                bindings::freshness(Some(&baseline), &checked)["status"],
                "UNRESOLVED"
            );
        }
        let service = service("python");
        let provider = fixture("python", PYTHON);
        let empty = source_for_provider_test(
            &service,
            &provider,
            &BTreeMap::from([("settings.py".into(), "LIMIT=1\n".into())]),
        );
        assert!(empty.entrypoints.is_empty());
        let checked = check::assemble(
            "input".into(),
            BTreeMap::from([("orders".into(), empty)]),
            BTreeMap::new(),
            &BTreeMap::new(),
            &BTreeMap::new(),
        )
        .unwrap();
        let baseline = render::make_bindings(&checked, BTreeMap::new()).unwrap();
        let changed = source_for_provider_test(
            &service,
            &provider,
            &BTreeMap::from([("settings.py".into(), "LIMIT=2\n".into())]),
        );
        let next = check::assemble(
            "input".into(),
            BTreeMap::from([("orders".into(), changed)]),
            BTreeMap::new(),
            &BTreeMap::new(),
            &BTreeMap::new(),
        )
        .unwrap();
        assert_eq!(
            bindings::freshness(Some(&baseline), &next)["status"],
            "STALE"
        );
    }
}
