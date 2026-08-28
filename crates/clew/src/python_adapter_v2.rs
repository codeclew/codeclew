use crate::adapter_v2::{
    ADAPTER_PROTOCOL, AdapterHandshake, AnalysisAttemptComplete, AnalysisEvent, AnalysisSink,
    AnalyzeGenerationRequest, CapabilityUri, FactRecord, FactShard, LanguageAdapter, LanguageUri,
    ToolchainConstraint,
};
use crate::canonical;
use crate::cas::{CasObject, CasStore};
use crate::error::{ClewError, ErrorCode};
use crate::python_project_model::{PYTHON_GRAMMAR_AUTHORITY, PythonCompilationSelector};
use crate::repository_snapshot::{RepositoryInputSnapshot, WorktreeKind};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use tree_sitter::{Node, Parser};

pub const PYTHON_LANGUAGE: &str = "language:python";
pub const PYTHON_SYNTAX_FACTS_CAPABILITY: &str = "analysis:python-syntax-facts";
pub const PYTHON_INDEX_SCHEMA: &str = "codeclew-python-syntax-index/2.0";
const FACT_SCHEMA: &str = "codeclew-python-syntax-fact/2.0";
const RECEIPT_SCHEMA: &str = "codeclew-python-syntax-completeness/1.0";
const ADAPTER_AUTHORITY_SCHEMA: &str = "codeclew-python-syntax-adapter/1.0";
pub(crate) const MAX_SOURCE_FILES: usize = 4096;
pub(crate) const MAX_SOURCE_FILE_BYTES: usize = 4 * 1024 * 1024;
pub(crate) const MAX_TOTAL_SOURCE_BYTES: usize = 128 * 1024 * 1024;
const MAX_FACTS: usize = 131_072;
const MAX_NESTING: usize = 64;
const MAX_NODES_PER_FILE: usize = 1_000_000;
const MAX_TOTAL_NODES: usize = 4_000_000;
const MAX_BOUNDARIES: usize = 4096;
const MAX_FACT_PAYLOAD_BYTES: usize = 64 * 1024;
const MAX_FACT_BATCH_INPUT_BYTES: usize = 128 * 1024 * 1024;
const MAX_FILE_IDENTIFIER_TERMS: usize = 2048;
const MAX_FILE_IDENTIFIER_TERM_BYTES: usize = 16 * 1024;

pub fn python_adapter_digest() -> Result<String, ClewError> {
    canonical::hash(&json!({
        "schema":ADAPTER_AUTHORITY_SCHEMA,
        "indexSchema":PYTHON_INDEX_SCHEMA,
        "factSchema":FACT_SCHEMA,
        "capability":PYTHON_SYNTAX_FACTS_CAPABILITY,
        "grammarAuthority":PYTHON_GRAMMAR_AUTHORITY,
    }))
    .map_err(internal)
}

pub fn python_scope_digest(index: &Value) -> Result<String, ClewError> {
    validate_index(index)?;
    canonical::hash(&json!({
        "schema":"codeclew-python-syntax-scope/1.0",
        "compilation":index["compilation"],
        "modelDigest":index["modelDigest"],
        "files":index["files"],
        "declarationDescriptors":index["declarationDescriptors"],
        "boundaries":index["boundaries"],
    }))
    .map_err(internal)
}

pub struct PythonAdapterV2 {
    adapter_digest: String,
    toolchain_digest: String,
    store: CasStore,
    index: Value,
    cancelled_attempts: Mutex<BTreeSet<String>>,
    stopped: AtomicBool,
}

impl PythonAdapterV2 {
    pub fn new(
        adapter_digest: String,
        toolchain_digest: String,
        store: CasStore,
        index: Value,
    ) -> Result<Self, ClewError> {
        require_digest(&adapter_digest)?;
        require_digest(&toolchain_digest)?;
        validate_index(&index)?;
        Ok(Self {
            adapter_digest,
            toolchain_digest,
            store,
            index,
            cancelled_attempts: Mutex::new(BTreeSet::new()),
            stopped: AtomicBool::new(false),
        })
    }
}

impl LanguageAdapter for PythonAdapterV2 {
    fn handshake(&self) -> Result<AdapterHandshake, ClewError> {
        Ok(AdapterHandshake {
            protocol: ADAPTER_PROTOCOL.into(),
            adapter_id: "python-syntax-1".into(),
            adapter_digest: self.adapter_digest.clone(),
            languages: vec![LanguageUri::parse(PYTHON_LANGUAGE)?],
            capabilities: vec![CapabilityUri::parse(PYTHON_SYNTAX_FACTS_CAPABILITY)?],
            toolchains: vec![ToolchainConstraint {
                authority_digest: self.toolchain_digest.clone(),
                minimum_version: None,
                maximum_version_exclusive: None,
            }],
        })
    }

    fn analyze_generation(
        &self,
        request: &AnalyzeGenerationRequest,
        sink: &mut dyn AnalysisSink,
        cancelled: &AtomicBool,
    ) -> Result<(), ClewError> {
        if self.stopped.load(Ordering::Acquire)
            || cancelled.load(Ordering::Acquire)
            || self
                .cancelled_attempts
                .lock()
                .map_err(poisoned)?
                .contains(&request.attempt_id)
        {
            return Err(cancelled_error());
        }
        if request.compilation.language_uri.as_str() != PYTHON_LANGUAGE
            || request.capability.as_str() != PYTHON_SYNTAX_FACTS_CAPABILITY
            || request.compilation.toolchain.digest != self.toolchain_digest
            || self.index.get("compilation").and_then(Value::as_str)
                != Some(request.compilation.compilation_id.as_str())
        {
            return Err(ClewError::new(
                ErrorCode::UnsupportedLanguage,
                "Python syntax request differs from its exact language/grammar authority",
            ));
        }
        let facts = translate_facts(&self.store, &self.index)?;
        let fact_count = facts.len() as u64;
        for (sequence, chunk) in facts.chunks(1024).enumerate() {
            if cancelled.load(Ordering::Acquire) {
                return Err(cancelled_error());
            }
            sink.accept(AnalysisEvent::FactShard(FactShard {
                sequence: u32::try_from(sequence)
                    .map_err(|_| resource("Python fact shard sequence overflow"))?,
                facts: chunk.to_vec(),
            }))?;
        }
        let scope_digest = python_scope_digest(&self.index)?;
        let receipt = self.store.put(
            RECEIPT_SCHEMA,
            &canonical::bytes(&json!({
                "schema":RECEIPT_SCHEMA,
                "scopeDigest":scope_digest,
                "coverage":"PARTIAL",
                "certainty":"UNSURE",
                "obligations":[
                    "VERIFY_PYTHON_RUNTIME_IMPORTS_AND_TYPES",
                    "VERIFY_DECORATORS_METACLAS_AND_DYNAMIC_EXECUTION"
                ],
            }))
            .map_err(internal)?,
        )?;
        sink.accept(AnalysisEvent::AttemptComplete(AnalysisAttemptComplete {
            scope_digest,
            completeness_receipt: receipt,
            fact_count,
        }))
    }

    fn cancel(&self, attempt_id: &str) -> Result<(), ClewError> {
        if attempt_id.is_empty() || attempt_id.len() > 128 {
            return Err(invalid("Python attempt identity is invalid"));
        }
        self.cancelled_attempts
            .lock()
            .map_err(poisoned)?
            .insert(attempt_id.into());
        Ok(())
    }

    fn shutdown(&self) -> Result<(), ClewError> {
        self.stopped.store(true, Ordering::Release);
        Ok(())
    }
}

pub fn translate_facts(store: &CasStore, index: &Value) -> Result<Vec<FactRecord>, ClewError> {
    validate_index(index)?;
    let capability = CapabilityUri::parse(PYTHON_SYNTAX_FACTS_CAPABILITY)?;
    let mut batch = PreparedFactBatch::default();
    for file in index["files"].as_array().expect("validated files") {
        batch.push(
            "source",
            json!({
                "schema":FACT_SCHEMA,
                "kind":"source-file",
                "path":file["path"],
                "module":file["module"],
                "contentHash":file["contentHash"],
                "identifierTerms":file["identifierTerms"],
                "identifierTermsTruncated":file["identifierTermsTruncated"],
                "resolution":"SOURCE_MEMBERSHIP_EXACT",
            }),
        )?;
    }
    for descriptor in index["declarationDescriptors"]["descriptors"]
        .as_array()
        .expect("validated descriptors")
    {
        let mut row = descriptor.clone();
        let object = row.as_object_mut().expect("validated descriptor");
        object.insert("schema".into(), Value::String(FACT_SCHEMA.into()));
        object.insert("kind".into(), Value::String("declaration".into()));
        batch.push("declaration", row)?;
    }
    for fact in index["syntaxFacts"].as_array().expect("validated facts") {
        let family = fact["kind"].as_str().expect("validated kind");
        let mut row = fact.clone();
        row.as_object_mut()
            .expect("validated fact")
            .insert("schema".into(), Value::String(FACT_SCHEMA.into()));
        batch.push(family, row)?;
    }
    for boundary in index["boundaries"]
        .as_array()
        .expect("validated boundaries")
    {
        batch.push(
            "boundary",
            json!({
                "schema":FACT_SCHEMA,
                "kind":"analysis-boundary",
                "code":boundary["code"],
                "file":boundary.get("file").cloned().unwrap_or(Value::Null),
                "subject":boundary.get("subject").cloned().unwrap_or(Value::Null),
                "resolution":"UNKNOWN",
            }),
        )?;
    }
    if batch.prepared.len() > MAX_FACTS {
        return Err(resource("Python syntax fact count exceeds its budget"));
    }
    let (fact_keys, inputs): (Vec<_>, Vec<_>) = batch
        .prepared
        .into_iter()
        .map(|(fact_key, bytes)| (fact_key, (FACT_SCHEMA.into(), bytes)))
        .unzip();
    let objects = store.put_batch(inputs)?;
    let mut facts = fact_keys
        .into_iter()
        .zip(objects)
        .map(|(fact_key, payload)| FactRecord {
            fact_key,
            domain_uri: capability.clone(),
            payload,
        })
        .collect::<Vec<_>>();
    facts.sort_by(|left, right| left.fact_key.cmp(&right.fact_key));
    Ok(facts)
}

#[derive(Default)]
struct PreparedFactBatch {
    prepared: Vec<(String, Vec<u8>)>,
    input_bytes: usize,
}

impl PreparedFactBatch {
    fn push(&mut self, family: &str, payload: Value) -> Result<(), ClewError> {
        let bytes = canonical::bytes(&payload).map_err(internal)?;
        if bytes.len() > MAX_FACT_PAYLOAD_BYTES {
            return Err(resource("Python syntax fact exceeds its payload budget"));
        }
        self.input_bytes = self
            .input_bytes
            .checked_add(bytes.len())
            .ok_or_else(|| resource("Python syntax fact batch size overflow"))?;
        if self.input_bytes > MAX_FACT_BATCH_INPUT_BYTES {
            return Err(resource(
                "Python syntax fact batch exceeds its input byte budget",
            ));
        }
        let digest = canonical::hash_bytes(&bytes);
        self.prepared
            .push((format!("python:{family}:{digest}"), bytes));
        Ok(())
    }
}

pub struct PythonSyntaxAuthority<'a> {
    pub compilation_id: &'a str,
    pub model_digest: &'a str,
    pub selector: &'a PythonCompilationSelector,
}

pub fn build_syntax_index(
    store: &CasStore,
    snapshot: &RepositoryInputSnapshot,
    authority: &PythonSyntaxAuthority<'_>,
) -> Result<Value, ClewError> {
    snapshot.verify()?;
    let inputs = effective_tracked_sources(snapshot, authority.selector)?;
    if inputs.is_empty() {
        return Err(ClewError::new(
            ErrorCode::UnsupportedLanguage,
            "Python selector contains no tracked source files",
        ));
    }
    if inputs.len() > MAX_SOURCE_FILES {
        return Err(resource("Python source file count exceeds its budget"));
    }
    let mut total_bytes = 0usize;
    for input in inputs.values() {
        if let SourceInput::Regular(object) = input {
            let size = usize::try_from(object.size)
                .map_err(|_| resource("Python source input exceeds host size"))?;
            if size > MAX_SOURCE_FILE_BYTES {
                return Err(resource("Python source file exceeds its byte budget"));
            }
            total_bytes = total_bytes
                .checked_add(size)
                .ok_or_else(|| resource("Python source byte count overflow"))?;
        }
    }
    if total_bytes > MAX_TOTAL_SOURCE_BYTES {
        return Err(resource("Python source set exceeds its byte budget"));
    }

    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_python::LANGUAGE.into())
        .map_err(|_| internal("pinned Python grammar is incompatible with the parser"))?;
    let mut files = Vec::new();
    let mut descriptors = BTreeMap::new();
    let mut syntax_facts = BTreeMap::new();
    let mut boundaries = BTreeSet::new();
    let mut node_budget = NodeBudget::new(MAX_NODES_PER_FILE, MAX_TOTAL_NODES);
    for (path, input) in &inputs {
        let module = authority.selector.module_name(path)?;
        match input {
            SourceInput::Symlink(object) => {
                files.push(json!({
                    "path":path,
                    "module":module,
                    "contentHash":object.digest,
                    "identifierTerms":[],
                    "identifierTermsTruncated":false,
                }));
                add_boundary(&mut boundaries, "PYTHON_SOURCE_SYMLINK", Some(path), None)?;
            }
            SourceInput::Regular(object) => {
                node_budget.start_file();
                let lease = store.read(object, MAX_SOURCE_FILE_BYTES)?;
                let bytes = lease.bytes();
                let Ok(source) = std::str::from_utf8(bytes) else {
                    files.push(json!({
                        "path":path,
                        "module":module,
                        "contentHash":canonical::hash_bytes(bytes),
                        "identifierTerms":[],
                        "identifierTermsTruncated":false,
                    }));
                    add_boundary(&mut boundaries, "PYTHON_SOURCE_NOT_UTF8", Some(path), None)?;
                    continue;
                };
                let tree = parser
                    .parse(source, None)
                    .ok_or_else(|| internal("pinned Python parser returned no syntax tree"))?;
                if tree.root_node().has_error() {
                    add_boundary(&mut boundaries, "PYTHON_PARSE_ERROR", Some(path), None)?;
                }
                let mut identifier_terms = IdentifierTerms::default();
                let mut context = SyntaxContext {
                    path,
                    module: &module,
                    source,
                    descriptors: &mut descriptors,
                    syntax_facts: &mut syntax_facts,
                    boundaries: &mut boundaries,
                    identifier_terms: &mut identifier_terms,
                };
                visit_node(tree.root_node(), &[], 0, &mut node_budget, &mut context)?;
                if identifier_terms.truncated {
                    add_boundary(
                        &mut boundaries,
                        "PYTHON_IDENTIFIER_TERMS_TRUNCATED",
                        Some(path),
                        None,
                    )?;
                }
                files.push(json!({
                    "path":path,
                    "module":module,
                    "contentHash":canonical::hash_bytes(bytes),
                    "identifierTerms":identifier_terms.terms,
                    "identifierTermsTruncated":identifier_terms.truncated,
                }));
            }
        }
    }
    for code in [
        "PYTHON_SYNTAX_ONLY",
        "PYTHON_IMPORT_RUNTIME_UNMODELED",
        "PYTHON_DYNAMIC_SEMANTICS_UNMODELED",
    ] {
        add_boundary(&mut boundaries, code, None, None)?;
    }
    if descriptors.len() + syntax_facts.len() + files.len() + boundaries.len() > MAX_FACTS {
        return Err(resource("Python syntax fact count exceeds its budget"));
    }
    let index = json!({
        "schema":PYTHON_INDEX_SCHEMA,
        "compilation":authority.compilation_id,
        "modelDigest":authority.model_digest,
        "importRoot":authority.selector.import_root,
        "sourceRoot":authority.selector.source_root,
        "grammarAuthority":PYTHON_GRAMMAR_AUTHORITY,
        "analysisCoverage":"PARTIAL",
        "analysisCertainty":"UNSURE",
        "files":files,
        "declarationDescriptors":{
            "coverage":"PARTIAL",
            "descriptors":descriptors.into_values().collect::<Vec<_>>(),
        },
        "declarationRelations":{"coverage":"PARTIAL","relations":[]},
        "syntaxFacts":syntax_facts.into_values().collect::<Vec<_>>(),
        "boundaries":boundaries
            .into_iter()
            .map(|encoded| serde_json::from_str::<Value>(&encoded).expect("canonical boundary"))
            .collect::<Vec<_>>(),
    });
    validate_index(&index)?;
    Ok(index)
}

#[derive(Clone)]
enum SourceInput {
    Regular(CasObject),
    Symlink(CasObject),
}

fn effective_tracked_sources(
    snapshot: &RepositoryInputSnapshot,
    selector: &PythonCompilationSelector,
) -> Result<BTreeMap<String, SourceInput>, ClewError> {
    if snapshot
        .index
        .iter()
        .any(|entry| entry.stage != 0 && selector.contains(&entry.path))
    {
        return Err(invalid(
            "selected Python source has an unmerged Git index entry",
        ));
    }
    let mut sources = snapshot
        .index
        .iter()
        .filter(|entry| entry.stage == 0 && selector.contains(&entry.path))
        .map(|entry| {
            let input = if entry.mode == 0o120000 {
                SourceInput::Symlink(entry.content.clone())
            } else {
                SourceInput::Regular(entry.content.clone())
            };
            (entry.path.clone(), input)
        })
        .collect::<BTreeMap<_, _>>();
    for entry in snapshot
        .worktree
        .iter()
        .filter(|entry| selector.contains(&entry.path))
    {
        if !sources.contains_key(&entry.path) {
            continue;
        }
        match entry.kind {
            WorktreeKind::Missing => {
                sources.remove(&entry.path);
            }
            WorktreeKind::Regular => {
                let content = entry
                    .content
                    .clone()
                    .ok_or_else(|| invalid("regular Python snapshot input has no content"))?;
                sources.insert(entry.path.clone(), SourceInput::Regular(content));
            }
            WorktreeKind::Symlink => {
                let content = entry
                    .content
                    .clone()
                    .ok_or_else(|| invalid("Python symlink snapshot input has no content"))?;
                sources.insert(entry.path.clone(), SourceInput::Symlink(content));
            }
        }
    }
    Ok(sources)
}

struct SyntaxContext<'a> {
    path: &'a str,
    module: &'a str,
    source: &'a str,
    descriptors: &'a mut BTreeMap<String, Value>,
    syntax_facts: &'a mut BTreeMap<String, Value>,
    boundaries: &'a mut BTreeSet<String>,
    identifier_terms: &'a mut IdentifierTerms,
}

#[derive(Default)]
struct IdentifierTerms {
    terms: BTreeSet<String>,
    bytes: usize,
    truncated: bool,
}

impl IdentifierTerms {
    fn observe(&mut self, value: &str) {
        if self.truncated || self.terms.contains(value) {
            return;
        }
        let next_bytes = self.bytes.saturating_add(value.len());
        if self.terms.len() == MAX_FILE_IDENTIFIER_TERMS
            || next_bytes > MAX_FILE_IDENTIFIER_TERM_BYTES
        {
            self.truncated = true;
            return;
        }
        self.bytes = next_bytes;
        self.terms.insert(value.to_owned());
    }
}

struct NodeBudget {
    file_nodes: usize,
    total_nodes: usize,
    max_file_nodes: usize,
    max_total_nodes: usize,
}

impl NodeBudget {
    fn new(max_file_nodes: usize, max_total_nodes: usize) -> Self {
        Self {
            file_nodes: 0,
            total_nodes: 0,
            max_file_nodes,
            max_total_nodes,
        }
    }

    fn start_file(&mut self) {
        self.file_nodes = 0;
    }

    fn visit(&mut self) -> Result<(), ClewError> {
        self.file_nodes = self
            .file_nodes
            .checked_add(1)
            .ok_or_else(|| resource("Python syntax node count overflow"))?;
        self.total_nodes = self
            .total_nodes
            .checked_add(1)
            .ok_or_else(|| resource("Python syntax node count overflow"))?;
        if self.file_nodes > self.max_file_nodes || self.total_nodes > self.max_total_nodes {
            return Err(resource("Python syntax node count exceeds its budget"));
        }
        Ok(())
    }
}

fn visit_node(
    node: Node<'_>,
    lexical_scope: &[String],
    depth: usize,
    node_budget: &mut NodeBudget,
    context: &mut SyntaxContext<'_>,
) -> Result<(), ClewError> {
    node_budget.visit()?;
    if node.kind() == "identifier" {
        let identifier = identifier_text(node, context.source)?;
        context.identifier_terms.observe(&identifier);
    }
    if depth > MAX_NESTING {
        add_boundary(
            context.boundaries,
            "PYTHON_NESTING_LIMIT",
            Some(context.path),
            None,
        )?;
        return Ok(());
    }
    if context.descriptors.len() + context.syntax_facts.len() >= MAX_FACTS {
        return Err(resource("Python syntax fact count exceeds its budget"));
    }

    let mut child_scope = lexical_scope.to_vec();
    match node.kind() {
        "class_definition" | "function_definition" => {
            let name_node = node
                .child_by_field_name("name")
                .ok_or_else(|| invalid("Python declaration has no name"))?;
            let name = identifier_text(name_node, context.source)?;
            let mut qualified = lexical_scope.to_vec();
            qualified.push(name.clone());
            let kind = if node.kind() == "class_definition" {
                "class"
            } else if has_async_token(node) {
                "async-function"
            } else if lexical_scope.is_empty() {
                "function"
            } else {
                "method-or-nested-function"
            };
            let identity = format!(
                "python-syntax:{}#{}:{}@{}-{}",
                context.path,
                kind,
                qualified.join("."),
                node.start_byte(),
                node.end_byte()
            );
            context.descriptors.insert(
                identity.clone(),
                json!({
                    "symbolIdentity":identity,
                    "name":name,
                    "qualifiedName":qualified.join("."),
                    "module":context.module,
                    "declarationKind":kind,
                    "file":context.path,
                    "rangeStart":node.start_byte(),
                    "rangeEnd":node.end_byte(),
                    "startLine":node.start_position().row + 1,
                    "endLine":node.end_position().row + 1,
                    "decorators":decorator_names(node, context.source)?,
                    "resolution":"SYNTAX_EXACT",
                }),
            );
            child_scope = qualified;
        }
        "import_statement" | "import_from_statement" => {
            let identifiers = import_identifiers(node, context.source)?;
            let row = json!({
                "kind":"import",
                "file":context.path,
                "module":context.module,
                "names":identifiers,
                "rangeStart":node.start_byte(),
                "rangeEnd":node.end_byte(),
                "startLine":node.start_position().row + 1,
                "endLine":node.end_position().row + 1,
                "resolution":"SYNTAX_ONLY",
            });
            context
                .syntax_facts
                .insert(canonical::hash(&row).map_err(internal)?, row);
        }
        "call" => {
            if let Some(function) = node.child_by_field_name("function")
                && let Some(callee) = dotted_identifier(function, context.source)?
            {
                let row = json!({
                    "kind":"call",
                    "file":context.path,
                    "module":context.module,
                    "callee":callee,
                    "lexicalOwner":lexical_scope.join("."),
                    "rangeStart":node.start_byte(),
                    "rangeEnd":node.end_byte(),
                    "startLine":node.start_position().row + 1,
                    "endLine":node.end_position().row + 1,
                    "resolution":"SYNTAX_ONLY",
                });
                context
                    .syntax_facts
                    .insert(canonical::hash(&row).map_err(internal)?, row);
            }
        }
        "decorator" => {
            if let Some(name) = decorator_identifier(node, context.source)? {
                let row = json!({
                    "kind":"decorator",
                    "file":context.path,
                    "module":context.module,
                    "name":name,
                    "rangeStart":node.start_byte(),
                    "rangeEnd":node.end_byte(),
                    "startLine":node.start_position().row + 1,
                    "endLine":node.end_position().row + 1,
                    "resolution":"SYNTAX_ONLY",
                });
                context
                    .syntax_facts
                    .insert(canonical::hash(&row).map_err(internal)?, row);
            }
        }
        "ERROR" => {
            add_boundary(
                context.boundaries,
                "PYTHON_PARSE_ERROR_NODE",
                Some(context.path),
                None,
            )?;
        }
        _ => {}
    }

    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        visit_node(child, &child_scope, depth + 1, node_budget, context)?;
    }
    Ok(())
}

fn decorator_names(node: Node<'_>, source: &str) -> Result<Vec<String>, ClewError> {
    let Some(parent) = node
        .parent()
        .filter(|parent| parent.kind() == "decorated_definition")
    else {
        return Ok(Vec::new());
    };
    let mut cursor = parent.walk();
    let mut names = parent
        .named_children(&mut cursor)
        .filter(|child| child.kind() == "decorator")
        .filter_map(|child| decorator_identifier(child, source).transpose())
        .collect::<Result<Vec<_>, _>>()?;
    names.sort();
    names.dedup();
    Ok(names)
}

fn decorator_identifier(node: Node<'_>, source: &str) -> Result<Option<String>, ClewError> {
    let mut cursor = node.walk();
    let expression = node
        .named_children(&mut cursor)
        .next()
        .filter(|child| child.kind() != "argument_list");
    let Some(mut expression) = expression else {
        return Ok(None);
    };
    if expression.kind() == "call"
        && let Some(function) = expression.child_by_field_name("function")
    {
        expression = function;
    }
    dotted_identifier(expression, source)
}

fn import_identifiers(node: Node<'_>, source: &str) -> Result<Vec<String>, ClewError> {
    let mut output = BTreeSet::new();
    let mut stack = vec![node];
    while let Some(current) = stack.pop() {
        if matches!(current.kind(), "identifier" | "dotted_name") {
            let text = current
                .utf8_text(source.as_bytes())
                .map_err(|_| invalid("Python import identifier is outside its UTF-8 source"))?;
            if safe_dotted_identifier(text) {
                output.insert(text.into());
            }
        }
        let mut cursor = current.walk();
        stack.extend(current.named_children(&mut cursor));
    }
    Ok(output.into_iter().collect())
}

fn dotted_identifier(node: Node<'_>, source: &str) -> Result<Option<String>, ClewError> {
    match node.kind() {
        "identifier" | "dotted_name" => {
            let text = node
                .utf8_text(source.as_bytes())
                .map_err(|_| invalid("Python identifier is outside its UTF-8 source"))?;
            Ok(safe_dotted_identifier(text).then(|| text.to_owned()))
        }
        "attribute" => {
            let Some(attribute) = node.child_by_field_name("attribute") else {
                return Ok(None);
            };
            let Some(name) = dotted_identifier(attribute, source)? else {
                return Ok(None);
            };
            let prefix = node
                .child_by_field_name("object")
                .and_then(|object| dotted_identifier(object, source).ok().flatten());
            Ok(Some(
                prefix.map_or(name.clone(), |prefix| format!("{prefix}.{name}")),
            ))
        }
        _ => Ok(None),
    }
}

fn identifier_text(node: Node<'_>, source: &str) -> Result<String, ClewError> {
    let text = node
        .utf8_text(source.as_bytes())
        .map_err(|_| invalid("Python declaration identifier is outside its UTF-8 source"))?;
    if !safe_identifier(text) {
        return Err(invalid("Python declaration identifier is invalid"));
    }
    Ok(text.into())
}

fn has_async_token(node: Node<'_>) -> bool {
    (0..node.child_count()).any(|index| {
        node.child(index)
            .is_some_and(|child| child.kind() == "async")
    })
}

fn add_boundary(
    boundaries: &mut BTreeSet<String>,
    code: &str,
    file: Option<&str>,
    subject: Option<&str>,
) -> Result<(), ClewError> {
    if boundaries.len() >= MAX_BOUNDARIES {
        return Err(resource(
            "Python analysis boundary count exceeds its budget",
        ));
    }
    let row = json!({
        "code":code,
        "file":file,
        "subject":subject,
        "resolution":"UNKNOWN",
    });
    boundaries.insert(
        String::from_utf8(canonical::bytes(&row).map_err(internal)?)
            .map_err(|_| internal("canonical boundary is not UTF-8"))?,
    );
    Ok(())
}

fn validate_index(index: &Value) -> Result<(), ClewError> {
    let files = index
        .get("files")
        .and_then(Value::as_array)
        .ok_or_else(|| invalid("Python syntax index has no file manifest"))?;
    let descriptors = index
        .pointer("/declarationDescriptors/descriptors")
        .and_then(Value::as_array)
        .ok_or_else(|| invalid("Python syntax index has no declarations"))?;
    let relations = index
        .pointer("/declarationRelations/relations")
        .and_then(Value::as_array)
        .ok_or_else(|| invalid("Python syntax index has no relation boundary"))?;
    let syntax_facts = index
        .get("syntaxFacts")
        .and_then(Value::as_array)
        .ok_or_else(|| invalid("Python syntax index has no syntax facts"))?;
    let boundaries = index
        .get("boundaries")
        .and_then(Value::as_array)
        .ok_or_else(|| invalid("Python syntax index has no analysis boundaries"))?;
    if index.get("schema").and_then(Value::as_str) != Some(PYTHON_INDEX_SCHEMA)
        || index.get("grammarAuthority").and_then(Value::as_str) != Some(PYTHON_GRAMMAR_AUTHORITY)
        || index.get("analysisCoverage").and_then(Value::as_str) != Some("PARTIAL")
        || index.get("analysisCertainty").and_then(Value::as_str) != Some("UNSURE")
        || index
            .pointer("/declarationDescriptors/coverage")
            .and_then(Value::as_str)
            != Some("PARTIAL")
        || index
            .pointer("/declarationRelations/coverage")
            .and_then(Value::as_str)
            != Some("PARTIAL")
        || files.is_empty()
        || files.len() > MAX_SOURCE_FILES
        || files
            .windows(2)
            .any(|pair| pair[0]["path"].as_str() >= pair[1]["path"].as_str())
        || files.iter().any(|file| {
            file.get("path")
                .and_then(Value::as_str)
                .is_none_or(|path| !safe_path(path) || !path.ends_with(".py"))
                || file.get("module").and_then(Value::as_str).is_none()
                || file
                    .get("contentHash")
                    .and_then(Value::as_str)
                    .is_none_or(|digest| require_digest(digest).is_err())
                || file
                    .get("identifierTerms")
                    .and_then(Value::as_array)
                    .is_none_or(|terms| {
                        terms.len() > MAX_FILE_IDENTIFIER_TERMS
                            || terms
                                .iter()
                                .any(|term| term.as_str().is_none_or(|term| !safe_identifier(term)))
                            || terms
                                .windows(2)
                                .any(|pair| pair[0].as_str() >= pair[1].as_str())
                    })
                || file
                    .get("identifierTermsTruncated")
                    .and_then(Value::as_bool)
                    .is_none()
        })
        || descriptors.len() + syntax_facts.len() + files.len() + boundaries.len() > MAX_FACTS
        || descriptors.iter().any(|row| !valid_located_fact(row, true))
        || descriptors
            .windows(2)
            .any(|pair| pair[0]["symbolIdentity"].as_str() >= pair[1]["symbolIdentity"].as_str())
        || !relations.is_empty()
        || syntax_facts.iter().any(|row| {
            !matches!(
                row.get("kind").and_then(Value::as_str),
                Some("import" | "call" | "decorator")
            ) || !valid_located_fact(row, false)
        })
        || boundaries.len() > MAX_BOUNDARIES
        || boundaries.iter().any(|row| {
            row.get("code").and_then(Value::as_str).is_none()
                || row.get("resolution").and_then(Value::as_str) != Some("UNKNOWN")
                || row
                    .get("file")
                    .and_then(Value::as_str)
                    .is_some_and(|path| !safe_path(path) || !path.ends_with(".py"))
        })
        || ["compilation", "modelDigest", "importRoot", "sourceRoot"]
            .iter()
            .any(|field| {
                index
                    .get(field)
                    .and_then(Value::as_str)
                    .is_none_or(str::is_empty)
            })
    {
        return Err(invalid("Python syntax index authority is invalid"));
    }
    Ok(())
}

fn valid_located_fact(row: &Value, declaration: bool) -> bool {
    row.get("file")
        .and_then(Value::as_str)
        .is_some_and(|path| safe_path(path) && path.ends_with(".py"))
        && row.get("module").and_then(Value::as_str).is_some()
        && row.get("rangeStart").and_then(Value::as_u64).is_some()
        && row.get("rangeEnd").and_then(Value::as_u64).is_some()
        && row["rangeStart"].as_u64() <= row["rangeEnd"].as_u64()
        && if declaration {
            row.get("symbolIdentity").and_then(Value::as_str).is_some()
                && row.get("name").and_then(Value::as_str).is_some()
                && row.get("declarationKind").and_then(Value::as_str).is_some()
                && row.get("resolution").and_then(Value::as_str) == Some("SYNTAX_EXACT")
        } else {
            row.get("resolution").and_then(Value::as_str) == Some("SYNTAX_ONLY")
        }
}

fn safe_identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 512
        && value
            .chars()
            .all(|character| character == '_' || character.is_alphanumeric())
}

fn safe_dotted_identifier(value: &str) -> bool {
    value.len() <= 1024 && value.split('.').all(safe_identifier)
}

fn safe_path(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 4096
        && !value.starts_with('/')
        && !value.contains(['\\', '\0'])
        && value
            .split('/')
            .all(|component| !component.is_empty() && component != "." && component != "..")
}

fn require_digest(value: &str) -> Result<(), ClewError> {
    if value.len() == 71
        && value.starts_with("sha256:")
        && value[7..]
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        Ok(())
    } else {
        Err(invalid("Python adapter authority digest is invalid"))
    }
}

fn cancelled_error() -> ClewError {
    ClewError::new(
        ErrorCode::TransactionRecoveryRequired,
        "Python analysis was cancelled",
    )
}

fn invalid(message: &str) -> ClewError {
    ClewError::new(ErrorCode::InvalidInput, message)
}

fn resource(message: &str) -> ClewError {
    ClewError::new(ErrorCode::ResourceLimit, message)
}

fn internal(error: impl std::fmt::Display) -> ClewError {
    ClewError::new(ErrorCode::Internal, error.to_string())
}

fn poisoned<T>(error: std::sync::PoisonError<T>) -> ClewError {
    ClewError::new(ErrorCode::Internal, error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repository_snapshot::{IndexEntry, SNAPSHOT_SCHEMA, WorktreeEntry};
    use crate::state::StateAuthority;
    use tempfile::TempDir;

    fn store() -> (TempDir, CasStore) {
        let root = tempfile::tempdir().unwrap();
        let state = StateAuthority::open(root.path().join("state")).unwrap();
        let store = CasStore::open(&state).unwrap();
        (root, store)
    }

    fn snapshot(store: &CasStore, rows: &[(&str, &[u8])]) -> RepositoryInputSnapshot {
        let mut index = rows
            .iter()
            .map(|(path, bytes)| IndexEntry {
                path: (*path).into(),
                mode: 0o100644,
                stage: 0,
                git_oid: "0".repeat(40),
                content: store
                    .put("codeclew-repository-input-blob/2.0", bytes)
                    .unwrap(),
            })
            .collect::<Vec<_>>();
        index.sort_by(|left, right| left.path.cmp(&right.path));
        let mut snapshot = RepositoryInputSnapshot {
            schema: SNAPSHOT_SCHEMA.into(),
            snapshot_id: String::new(),
            staged_view_digest: format!("sha256:{}", "1".repeat(64)),
            cached_view_digest: format!("sha256:{}", "2".repeat(64)),
            untracked_view_digest: format!("sha256:{}", "3".repeat(64)),
            index,
            worktree: Vec::new(),
        };
        snapshot.snapshot_id = canonical::hash(&snapshot).unwrap();
        snapshot
    }

    #[test]
    fn parser_indexes_declarations_imports_decorators_and_calls_without_literals() {
        let (_root, store) = store();
        let source = br#"
from lib.service import execute as run

@router.post("/private-route")
class Handler:
    async def apply(self):
        local_focus = self.current_focus
        return run()
"#;
        let snapshot = snapshot(&store, &[("backend/api.py", source)]);
        let selector = PythonCompilationSelector::parse("python:.#backend").unwrap();
        let index = build_syntax_index(
            &store,
            &snapshot,
            &PythonSyntaxAuthority {
                compilation_id: "python-root",
                model_digest: &format!("sha256:{}", "a".repeat(64)),
                selector: &selector,
            },
        )
        .unwrap();
        assert_eq!(
            index["declarationDescriptors"]["descriptors"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
        let encoded = canonical::bytes(&index).unwrap();
        let encoded = String::from_utf8(encoded).unwrap();
        assert!(encoded.contains("router.post"));
        assert!(encoded.contains("execute"));
        assert!(encoded.contains("run"));
        assert!(encoded.contains("local_focus"));
        assert!(encoded.contains("current_focus"));
        assert!(!encoded.contains("private-route"));
        let facts = translate_facts(&store, &index).unwrap();
        let source_fact = facts
            .iter()
            .find(|fact| fact.fact_key.starts_with("python:source:"))
            .unwrap();
        let source_payload = store
            .read(
                &source_fact.payload,
                usize::try_from(source_fact.payload.size).unwrap(),
            )
            .unwrap();
        let source_payload = String::from_utf8(source_payload.bytes().to_vec()).unwrap();
        assert!(source_payload.contains("local_focus"));
        assert!(!source_payload.contains("private-route"));
    }

    #[test]
    fn dirty_tracked_overlay_wins_and_untracked_python_is_excluded() {
        let (_root, store) = store();
        let mut snapshot = snapshot(&store, &[("src/tracked.py", b"def old(): pass\n")]);
        snapshot.worktree = vec![
            WorktreeEntry {
                path: "src/tracked.py".into(),
                kind: WorktreeKind::Regular,
                mode: 0o100644,
                content: Some(
                    store
                        .put(
                            "codeclew-repository-input-blob/2.0",
                            b"def replacement(): pass\n",
                        )
                        .unwrap(),
                ),
            },
            WorktreeEntry {
                path: "src/untracked.py".into(),
                kind: WorktreeKind::Regular,
                mode: 0o100644,
                content: Some(
                    store
                        .put(
                            "codeclew-repository-input-blob/2.0",
                            b"def poison(): pass\n",
                        )
                        .unwrap(),
                ),
            },
        ];
        snapshot.snapshot_id.clear();
        snapshot.snapshot_id = canonical::hash(&snapshot).unwrap();
        let selector = PythonCompilationSelector::parse("python:.#src").unwrap();
        let index = build_syntax_index(
            &store,
            &snapshot,
            &PythonSyntaxAuthority {
                compilation_id: "python-root",
                model_digest: &format!("sha256:{}", "a".repeat(64)),
                selector: &selector,
            },
        )
        .unwrap();
        let encoded = String::from_utf8(canonical::bytes(&index).unwrap()).unwrap();
        assert!(encoded.contains("replacement"));
        assert!(!encoded.contains("old"));
        assert!(!encoded.contains("poison"));
        assert!(!encoded.contains("untracked.py"));
    }

    #[test]
    fn non_utf8_and_syntax_errors_are_explicit_boundaries() {
        let (_root, store) = store();
        let snapshot = snapshot(
            &store,
            &[
                ("src/bad.py", b"def broken(:\n"),
                ("src/legacy.py", &[0xff, 0xfe]),
            ],
        );
        let selector = PythonCompilationSelector::parse("python:.#src").unwrap();
        let index = build_syntax_index(
            &store,
            &snapshot,
            &PythonSyntaxAuthority {
                compilation_id: "python-root",
                model_digest: &format!("sha256:{}", "a".repeat(64)),
                selector: &selector,
            },
        )
        .unwrap();
        let encoded = String::from_utf8(canonical::bytes(&index).unwrap()).unwrap();
        assert!(encoded.contains("PYTHON_PARSE_ERROR"));
        assert!(encoded.contains("PYTHON_SOURCE_NOT_UTF8"));
    }

    #[test]
    fn deleted_sources_disappear_and_symlinks_are_never_parsed() {
        let (_root, store) = store();
        let mut snapshot = snapshot(
            &store,
            &[
                ("src/deleted.py", b"def deleted(): pass\n"),
                ("src/link.py", b"outside.py"),
            ],
        );
        snapshot.index[1].mode = 0o120000;
        snapshot.worktree = vec![WorktreeEntry {
            path: "src/deleted.py".into(),
            kind: WorktreeKind::Missing,
            mode: 0,
            content: None,
        }];
        snapshot.snapshot_id.clear();
        snapshot.snapshot_id = canonical::hash(&snapshot).unwrap();
        let selector = PythonCompilationSelector::parse("python:.#src").unwrap();
        let index = build_syntax_index(
            &store,
            &snapshot,
            &PythonSyntaxAuthority {
                compilation_id: "python-root",
                model_digest: &format!("sha256:{}", "a".repeat(64)),
                selector: &selector,
            },
        )
        .unwrap();
        let encoded = String::from_utf8(canonical::bytes(&index).unwrap()).unwrap();
        assert!(!encoded.contains("deleted.py"));
        assert!(encoded.contains("link.py"));
        assert!(encoded.contains("PYTHON_SOURCE_SYMLINK"));
        assert!(!encoded.contains("outside.py"));
    }

    #[test]
    fn oversized_source_fails_before_fact_publication() {
        let (_root, store) = store();
        let oversized = vec![b'x'; MAX_SOURCE_FILE_BYTES + 1];
        let snapshot = snapshot(&store, &[("src/oversized.py", oversized.as_slice())]);
        let selector = PythonCompilationSelector::parse("python:.#src").unwrap();
        let error = build_syntax_index(
            &store,
            &snapshot,
            &PythonSyntaxAuthority {
                compilation_id: "python-root",
                model_digest: &format!("sha256:{}", "a".repeat(64)),
                selector: &selector,
            },
        )
        .unwrap_err();
        assert_eq!(error.code, ErrorCode::ResourceLimit);
    }

    #[test]
    fn syntax_node_budget_fails_atomically() {
        let mut parser = Parser::new();
        parser
            .set_language(&tree_sitter_python::LANGUAGE.into())
            .unwrap();
        let source = "first = one()\nsecond = two()\n";
        let tree = parser.parse(source, None).unwrap();
        let mut descriptors = BTreeMap::new();
        let mut syntax_facts = BTreeMap::new();
        let mut boundaries = BTreeSet::new();
        let mut identifier_terms = IdentifierTerms::default();
        let mut context = SyntaxContext {
            path: "src/bounded.py",
            module: "src.bounded",
            source,
            descriptors: &mut descriptors,
            syntax_facts: &mut syntax_facts,
            boundaries: &mut boundaries,
            identifier_terms: &mut identifier_terms,
        };
        let mut budget = NodeBudget::new(2, 2);
        budget.start_file();
        let error = visit_node(tree.root_node(), &[], 0, &mut budget, &mut context).unwrap_err();
        assert_eq!(error.code, ErrorCode::ResourceLimit);
    }

    #[test]
    fn generic_mixed_fixture_is_parsed_without_executing_poison_sources() {
        let (_root, store) = store();
        let snapshot = snapshot(
            &store,
            &[
                (
                    "fixtures/python-mixed/src/example/__init__.py",
                    include_bytes!("../../../fixtures/python-mixed/src/example/__init__.py"),
                ),
                (
                    "fixtures/python-mixed/src/example/service.py",
                    include_bytes!("../../../fixtures/python-mixed/src/example/service.py"),
                ),
                (
                    "fixtures/python-mixed/src/example/support.py",
                    include_bytes!("../../../fixtures/python-mixed/src/example/support.py"),
                ),
                (
                    "fixtures/python-mixed/src/sitecustomize.py",
                    include_bytes!("../../../fixtures/python-mixed/src/sitecustomize.py"),
                ),
            ],
        );
        let selector = PythonCompilationSelector::parse(
            "python:fixtures/python-mixed#fixtures/python-mixed/src",
        )
        .unwrap();
        let index = build_syntax_index(
            &store,
            &snapshot,
            &PythonSyntaxAuthority {
                compilation_id: "python-fixture",
                model_digest: &format!("sha256:{}", "a".repeat(64)),
                selector: &selector,
            },
        )
        .unwrap();
        let encoded = String::from_utf8(canonical::bytes(&index).unwrap()).unwrap();
        assert!(encoded.contains("Service"));
        assert!(encoded.contains("normalize"));
        assert!(encoded.contains("RuntimeError"));
        assert!(!encoded.contains("Codeclew must never execute"));
        assert!(!encoded.contains("Python analysis must not start"));
    }
}
