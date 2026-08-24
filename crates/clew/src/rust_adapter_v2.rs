use crate::adapter_v2::{
    ADAPTER_PROTOCOL, AdapterHandshake, AnalysisAttemptComplete, AnalysisEvent, AnalysisSink,
    AnalyzeGenerationRequest, CapabilityUri, FactRecord, FactShard, LanguageAdapter, LanguageUri,
    ToolchainConstraint,
};
use crate::canonical;
use crate::cas::CasStore;
use crate::error::{ClewError, ErrorCode};
use crate::repository_snapshot::{RepositoryInputSnapshot, WorktreeKind};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use syn::spanned::Spanned;

pub const RUST_LANGUAGE: &str = "language:rust";
pub const RUST_SYNTAX_FACTS_CAPABILITY: &str = "analysis:rust-syntax-facts";
pub const RUST_INDEX_SCHEMA: &str = "codeclew-rust-syntax-index/1.0";
const FACT_SCHEMA: &str = "codeclew-rust-syntax-fact/1.0";
const RECEIPT_SCHEMA: &str = "codeclew-rust-syntax-completeness/1.0";
const ADAPTER_AUTHORITY_SCHEMA: &str = "codeclew-rust-syntax-adapter/1.0";
const PARSER_AUTHORITY: &str = "syn-2.0.119/full+proc-macro2-1.0.107/span-locations";
const MAX_SOURCE_FILES: usize = 512;
const MAX_SOURCE_FILE_BYTES: usize = 2 * 1024 * 1024;
const MAX_TOTAL_SOURCE_BYTES: usize = 32 * 1024 * 1024;
const MAX_DECLARATIONS: usize = 65_536;
const MAX_NESTING: usize = 64;
const MAX_FACT_PAYLOAD_BYTES: usize = 64 * 1024;
const MAX_FACT_BATCH_INPUT_BYTES: usize = 128 * 1024 * 1024;
const MAX_BOUNDARIES: usize = 4096;

pub fn rust_adapter_digest() -> Result<String, ClewError> {
    canonical::hash(&json!({
        "schema":ADAPTER_AUTHORITY_SCHEMA,
        "indexSchema":RUST_INDEX_SCHEMA,
        "factSchema":FACT_SCHEMA,
        "capability":RUST_SYNTAX_FACTS_CAPABILITY,
        "parserAuthority":PARSER_AUTHORITY,
    }))
    .map_err(internal)
}

pub fn rust_scope_digest(index: &Value) -> Result<String, ClewError> {
    validate_index(index)?;
    canonical::hash(&json!({
        "schema":"codeclew-rust-syntax-scope/1.0",
        "compilation":index["compilation"],
        "modelDigest":index["modelDigest"],
        "files":index["files"],
        "declarationDescriptors":index["declarationDescriptors"],
        "boundaries":index["boundaries"],
    }))
    .map_err(internal)
}

pub struct RustAdapterV2 {
    adapter_digest: String,
    toolchain_digest: String,
    store: CasStore,
    index: Value,
    cancelled_attempts: Mutex<BTreeSet<String>>,
    stopped: AtomicBool,
}

impl RustAdapterV2 {
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

impl LanguageAdapter for RustAdapterV2 {
    fn handshake(&self) -> Result<AdapterHandshake, ClewError> {
        Ok(AdapterHandshake {
            protocol: ADAPTER_PROTOCOL.into(),
            adapter_id: "rust-syntax-1".into(),
            adapter_digest: self.adapter_digest.clone(),
            languages: vec![LanguageUri::parse(RUST_LANGUAGE)?],
            capabilities: vec![CapabilityUri::parse(RUST_SYNTAX_FACTS_CAPABILITY)?],
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
        if request.compilation.language_uri.as_str() != RUST_LANGUAGE
            || request.capability.as_str() != RUST_SYNTAX_FACTS_CAPABILITY
            || request.compilation.toolchain.digest != self.toolchain_digest
            || self.index.get("compilation").and_then(Value::as_str)
                != Some(request.compilation.compilation_id.as_str())
        {
            return Err(ClewError::new(
                ErrorCode::UnsupportedLanguage,
                "Rust syntax request differs from its exact language/toolchain authority",
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
                    .map_err(|_| resource("Rust fact shard sequence overflow"))?,
                facts: chunk.to_vec(),
            }))?;
        }
        let scope_digest = rust_scope_digest(&self.index)?;
        let receipt = self.store.put(
            RECEIPT_SCHEMA,
            &canonical::bytes(&json!({
                "schema":RECEIPT_SCHEMA,
                "scopeDigest":scope_digest,
                "coverage":"PARTIAL",
                "certainty":"UNSURE",
                "obligations":["VERIFY_RUST_NAME_RESOLUTION","VERIFY_CFG_AND_MACRO_EXPANSION"],
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
            return Err(invalid("Rust attempt identity is invalid"));
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
    let capability = CapabilityUri::parse(RUST_SYNTAX_FACTS_CAPABILITY)?;
    let mut batch = PreparedFactBatch::default();
    batch.push(
        json!({
            "schema":FACT_SCHEMA,
            "kind":"cargo-target",
            "package":index["package"],
            "targetKind":index["targetKind"],
            "targetName":index["targetName"],
            "sourcePath":index["sourcePath"],
            "cargoVersion":index["cargoVersion"],
            "rustcVersion":index["rustcVersion"],
            "resolution":"CARGO_MODEL_EXACT",
        }),
        MAX_FACT_BATCH_INPUT_BYTES,
    )?;
    for file in index["files"].as_array().expect("validated files") {
        batch.push(
            json!({
                "schema":FACT_SCHEMA,
                "kind":"source-file",
                "path":file["path"],
                "contentHash":file["contentHash"],
                "package":index["package"],
                "targetName":index["targetName"],
                "resolution":"SOURCE_MEMBERSHIP_EXACT",
            }),
            MAX_FACT_BATCH_INPUT_BYTES,
        )?;
    }
    for descriptor in index["declarationDescriptors"]["descriptors"]
        .as_array()
        .expect("validated descriptors")
    {
        let mut row = descriptor.clone();
        row.as_object_mut()
            .expect("validated descriptor object")
            .insert("schema".into(), Value::String(FACT_SCHEMA.into()));
        row.as_object_mut()
            .expect("validated descriptor object")
            .insert("kind".into(), Value::String("declaration".into()));
        batch.push(row, MAX_FACT_BATCH_INPUT_BYTES)?;
    }
    for boundary in index["boundaries"]
        .as_array()
        .expect("validated boundaries")
    {
        batch.push(
            json!({
                "schema":FACT_SCHEMA,
                "kind":"analysis-boundary",
                "code":boundary["code"],
                "file":boundary.get("file").cloned().unwrap_or(Value::Null),
                "subject":boundary.get("subject").cloned().unwrap_or(Value::Null),
                "resolution":"UNKNOWN",
            }),
            MAX_FACT_BATCH_INPUT_BYTES,
        )?;
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
    fn push(&mut self, payload: Value, input_limit: usize) -> Result<(), ClewError> {
        let bytes = canonical::bytes(&payload).map_err(internal)?;
        if bytes.len() > MAX_FACT_PAYLOAD_BYTES {
            return Err(resource("Rust syntax fact exceeds its payload budget"));
        }
        let next = self
            .input_bytes
            .checked_add(bytes.len())
            .ok_or_else(|| resource("Rust syntax fact batch size overflow"))?;
        if next > input_limit {
            return Err(resource(
                "Rust syntax fact batch exceeds its input byte budget",
            ));
        }
        let digest = canonical::hash_bytes(&bytes);
        self.prepared.push((format!("rust-syntax:{digest}"), bytes));
        self.input_bytes = next;
        Ok(())
    }
}

fn validate_index(index: &Value) -> Result<(), ClewError> {
    let files = index
        .get("files")
        .and_then(Value::as_array)
        .ok_or_else(|| invalid("Rust syntax index has no file manifest"))?;
    let descriptors = index
        .pointer("/declarationDescriptors/descriptors")
        .and_then(Value::as_array)
        .ok_or_else(|| invalid("Rust syntax index has no declaration descriptors"))?;
    let relations = index
        .pointer("/declarationRelations/relations")
        .and_then(Value::as_array)
        .ok_or_else(|| invalid("Rust syntax index has no relation boundary"))?;
    let boundaries = index
        .get("boundaries")
        .and_then(Value::as_array)
        .ok_or_else(|| invalid("Rust syntax index has no analysis boundaries"))?;
    if index.get("schema").and_then(Value::as_str) != Some(RUST_INDEX_SCHEMA)
        || index.get("analysisCoverage").and_then(Value::as_str) != Some("PARTIAL")
        || index.get("analysisCertainty").and_then(Value::as_str) != Some("UNSURE")
        || index.get("parserAuthority").and_then(Value::as_str) != Some(PARSER_AUTHORITY)
        || index
            .pointer("/declarationDescriptors/coverage")
            .and_then(Value::as_str)
            != Some("PARTIAL")
        || index
            .pointer("/declarationRelations/coverage")
            .and_then(Value::as_str)
            != Some("PARTIAL")
        || files.is_empty()
        || files.len() > 4096
        || files.iter().any(|file| {
            file.get("path")
                .and_then(Value::as_str)
                .is_none_or(|path| !safe_path(path) || !path.ends_with(".rs"))
                || file
                    .get("contentHash")
                    .and_then(Value::as_str)
                    .is_none_or(|digest| require_digest(digest).is_err())
        })
        || files
            .windows(2)
            .any(|pair| pair[0]["path"].as_str() >= pair[1]["path"].as_str())
        || descriptors.len() > MAX_DECLARATIONS
        || descriptors.iter().any(|descriptor| {
            descriptor
                .get("symbolIdentity")
                .and_then(Value::as_str)
                .is_none()
                || descriptor.get("name").and_then(Value::as_str).is_none()
                || descriptor
                    .get("declarationKind")
                    .and_then(Value::as_str)
                    .is_none()
                || descriptor
                    .get("file")
                    .and_then(Value::as_str)
                    .is_none_or(|path| !safe_path(path) || !path.ends_with(".rs"))
                || descriptor.get("resolution").and_then(Value::as_str) != Some("SYNTAX_EXACT")
                || descriptor
                    .get("rangeStart")
                    .and_then(Value::as_u64)
                    .is_none()
                || descriptor.get("rangeEnd").and_then(Value::as_u64).is_none()
                || descriptor["rangeStart"].as_u64() > descriptor["rangeEnd"].as_u64()
        })
        || descriptors
            .windows(2)
            .any(|pair| pair[0]["symbolIdentity"].as_str() >= pair[1]["symbolIdentity"].as_str())
        || !relations.is_empty()
        || boundaries.len() > MAX_BOUNDARIES
        || boundaries.iter().any(|boundary| {
            boundary.get("code").and_then(Value::as_str).is_none()
                || boundary.get("resolution").and_then(Value::as_str) != Some("UNKNOWN")
                || boundary
                    .get("file")
                    .and_then(Value::as_str)
                    .is_some_and(|path| !safe_path(path) || !path.ends_with(".rs"))
        })
        || [
            "compilation",
            "modelDigest",
            "package",
            "targetKind",
            "targetName",
            "sourcePath",
            "cargoVersion",
            "rustcVersion",
        ]
        .iter()
        .any(|field| {
            index
                .get(field)
                .and_then(Value::as_str)
                .is_none_or(str::is_empty)
        })
    {
        return Err(invalid("Rust syntax index authority is invalid"));
    }
    Ok(())
}

pub struct RustSyntaxAuthority<'a> {
    pub compilation_id: &'a str,
    pub model_digest: &'a str,
    pub package: &'a str,
    pub target_kind: &'a str,
    pub target_name: &'a str,
    pub source_path: &'a str,
    pub cargo_version: &'a str,
    pub rustc_version: &'a str,
}

pub fn build_syntax_index(
    store: &CasStore,
    snapshot: &RepositoryInputSnapshot,
    authority: &RustSyntaxAuthority<'_>,
) -> Result<Value, ClewError> {
    snapshot.verify()?;
    let sources = effective_sources(snapshot)?;
    if !sources.contains_key(authority.source_path) {
        return Err(invalid(
            "Cargo target source is absent from the repository snapshot",
        ));
    }
    let mut queue = VecDeque::from([(
        authority.source_path.to_owned(),
        root_module_dir(authority.source_path)?,
    )]);
    let mut visited = BTreeSet::new();
    let mut files = BTreeMap::new();
    let mut descriptors = BTreeMap::new();
    let mut boundaries = BTreeSet::new();
    let mut total_bytes = 0usize;
    while let Some((path, module_dir)) = queue.pop_front() {
        if visited.contains(&path) {
            continue;
        }
        if visited.len() >= MAX_SOURCE_FILES {
            boundary(&mut boundaries, "RUST_SOURCE_FILE_LIMIT", None, None);
            break;
        }
        visited.insert(path.clone());
        let object = sources
            .get(&path)
            .ok_or_else(|| invalid("selected Rust module disappeared from its snapshot"))?;
        let size = usize::try_from(object.size)
            .map_err(|_| resource("Rust source input exceeds host size"))?;
        if size > MAX_SOURCE_FILE_BYTES {
            boundary(
                &mut boundaries,
                "RUST_SOURCE_FILE_TOO_LARGE",
                Some(&path),
                None,
            );
            continue;
        }
        if total_bytes.saturating_add(size) > MAX_TOTAL_SOURCE_BYTES {
            boundary(
                &mut boundaries,
                "RUST_TOTAL_SOURCE_LIMIT",
                Some(&path),
                None,
            );
            break;
        }
        let lease = store.read(object, MAX_SOURCE_FILE_BYTES)?;
        let bytes = lease.bytes();
        total_bytes += bytes.len();
        let content_hash = canonical::hash_bytes(bytes);
        files.insert(
            path.clone(),
            json!({"path":path,"contentHash":content_hash}),
        );
        let source = match std::str::from_utf8(bytes) {
            Ok(source) => source,
            Err(_) => {
                boundary(&mut boundaries, "RUST_SOURCE_NOT_UTF8", Some(&path), None);
                continue;
            }
        };
        let syntax = match syn::parse_file(source) {
            Ok(syntax) => syntax,
            Err(_) => {
                boundary(&mut boundaries, "RUST_PARSE_FAILED", Some(&path), None);
                continue;
            }
        };
        let line_starts = line_starts(source);
        let mut context = SyntaxContext {
            path: &path,
            source,
            line_starts: &line_starts,
            sources: &sources,
            descriptors: &mut descriptors,
            boundaries: &mut boundaries,
            queue: &mut queue,
        };
        visit_items(&syntax.items, &module_dir, 0, &mut context)?;
    }
    let index = json!({
        "schema":RUST_INDEX_SCHEMA,
        "compilation":authority.compilation_id,
        "modelDigest":authority.model_digest,
        "package":authority.package,
        "targetKind":authority.target_kind,
        "targetName":authority.target_name,
        "sourcePath":authority.source_path,
        "cargoVersion":authority.cargo_version,
        "rustcVersion":authority.rustc_version,
        "parserAuthority":PARSER_AUTHORITY,
        "analysisCoverage":"PARTIAL",
        "analysisCertainty":"UNSURE",
        "files":files.into_values().collect::<Vec<_>>(),
        "declarationDescriptors":{
            "coverage":"PARTIAL",
            "descriptors":descriptors.into_values().collect::<Vec<_>>()
        },
        "declarationRelations":{"coverage":"PARTIAL","relations":[]},
        "boundaries":boundaries
            .into_iter()
            .map(|encoded| serde_json::from_str::<Value>(&encoded).expect("canonical boundary"))
            .collect::<Vec<_>>(),
    });
    validate_index(&index)?;
    Ok(index)
}

fn effective_sources(
    snapshot: &RepositoryInputSnapshot,
) -> Result<BTreeMap<String, crate::cas::CasObject>, ClewError> {
    let mut sources = snapshot
        .index
        .iter()
        .filter(|entry| entry.stage == 0 && entry.path.ends_with(".rs"))
        .map(|entry| (entry.path.clone(), entry.content.clone()))
        .collect::<BTreeMap<_, _>>();
    for entry in snapshot
        .worktree
        .iter()
        .filter(|entry| entry.path.ends_with(".rs"))
    {
        match entry.kind {
            WorktreeKind::Missing => {
                sources.remove(&entry.path);
            }
            WorktreeKind::Regular => {
                let content = entry
                    .content
                    .clone()
                    .ok_or_else(|| invalid("regular Rust snapshot input has no content"))?;
                sources.insert(entry.path.clone(), content);
            }
            WorktreeKind::Symlink => {
                sources.remove(&entry.path);
            }
        }
    }
    Ok(sources)
}

struct SyntaxContext<'a> {
    path: &'a str,
    source: &'a str,
    line_starts: &'a [usize],
    sources: &'a BTreeMap<String, crate::cas::CasObject>,
    descriptors: &'a mut BTreeMap<String, Value>,
    boundaries: &'a mut BTreeSet<String>,
    queue: &'a mut VecDeque<(String, String)>,
}

fn visit_items(
    items: &[syn::Item],
    module_dir: &str,
    depth: usize,
    context: &mut SyntaxContext<'_>,
) -> Result<(), ClewError> {
    if depth > MAX_NESTING {
        boundary(
            context.boundaries,
            "RUST_NESTING_LIMIT",
            Some(context.path),
            None,
        );
        return Ok(());
    }
    for item in items {
        if context.descriptors.len() >= MAX_DECLARATIONS {
            boundary(context.boundaries, "RUST_DECLARATION_LIMIT", None, None);
            return Ok(());
        }
        match item {
            syn::Item::Fn(value) => {
                add_declaration("function", &value.sig.ident, value, &value.attrs, context)?
            }
            syn::Item::Struct(value) => {
                add_declaration("struct", &value.ident, value, &value.attrs, context)?
            }
            syn::Item::Enum(value) => {
                add_declaration("enum", &value.ident, value, &value.attrs, context)?
            }
            syn::Item::Trait(value) => {
                add_declaration("trait", &value.ident, value, &value.attrs, context)?;
                for member in &value.items {
                    if let syn::TraitItem::Fn(method) = member {
                        add_declaration(
                            "trait-method",
                            &method.sig.ident,
                            method,
                            &method.attrs,
                            context,
                        )?;
                    }
                }
            }
            syn::Item::Type(value) => {
                add_declaration("type-alias", &value.ident, value, &value.attrs, context)?
            }
            syn::Item::Const(value) => {
                add_declaration("const", &value.ident, value, &value.attrs, context)?
            }
            syn::Item::Static(value) => {
                add_declaration("static", &value.ident, value, &value.attrs, context)?
            }
            syn::Item::Impl(value) => {
                note_attributes(&value.attrs, context, "impl");
                for member in &value.items {
                    if let syn::ImplItem::Fn(method) = member {
                        add_declaration(
                            "impl-method",
                            &method.sig.ident,
                            method,
                            &method.attrs,
                            context,
                        )?;
                    }
                }
            }
            syn::Item::Mod(value) => {
                add_declaration("module", &value.ident, value, &value.attrs, context)?;
                if let Some((_, nested)) = &value.content {
                    let nested_dir = join_path(module_dir, &value.ident.to_string())?;
                    visit_items(nested, &nested_dir, depth + 1, context)?;
                } else {
                    resolve_external_module(value, module_dir, context)?;
                }
            }
            syn::Item::Macro(_) => boundary(
                context.boundaries,
                "RUST_MACRO_ITEM_NOT_EXPANDED",
                Some(context.path),
                None,
            ),
            _ => note_attributes(item_attrs(item), context, "item"),
        }
    }
    Ok(())
}

fn add_declaration(
    kind: &str,
    ident: &syn::Ident,
    spanned: &impl Spanned,
    attrs: &[syn::Attribute],
    context: &mut SyntaxContext<'_>,
) -> Result<(), ClewError> {
    note_attributes(attrs, context, ident.to_string().as_str());
    let span = spanned.span();
    let start = offset(context.source, context.line_starts, span.start())?;
    let end = offset(context.source, context.line_starts, span.end())?;
    let name = ident.to_string();
    let identity = format!(
        "rust-syntax:{}#{}:{}@{}-{}",
        context.path, kind, name, start, end
    );
    context.descriptors.insert(
        identity.clone(),
        json!({
            "symbolIdentity":identity,
            "name":name,
            "declarationKind":kind,
            "file":context.path,
            "rangeStart":start,
            "rangeEnd":end,
            "startLine":span.start().line,
            "endLine":span.end().line,
        "cfgStatus":if has_any_attribute(attrs, &["cfg", "cfg_attr"]) { "UNKNOWN" } else { "UNCONDITIONAL" },
            "resolution":"SYNTAX_EXACT",
        }),
    );
    Ok(())
}

fn resolve_external_module(
    module: &syn::ItemMod,
    module_dir: &str,
    context: &mut SyntaxContext<'_>,
) -> Result<(), ClewError> {
    let name = module.ident.to_string();
    if has_attribute(&module.attrs, "path") {
        boundary(
            context.boundaries,
            "RUST_CUSTOM_MODULE_PATH",
            Some(context.path),
            Some(&name),
        );
        return Ok(());
    }
    if has_attribute(&module.attrs, "cfg") {
        boundary(
            context.boundaries,
            "RUST_CFG_MODULE_NOT_EXPANDED",
            Some(context.path),
            Some(&name),
        );
        return Ok(());
    }
    if has_attribute(&module.attrs, "cfg_attr") {
        boundary(
            context.boundaries,
            "RUST_CFG_ATTR_MODULE_NOT_EXPANDED",
            Some(context.path),
            Some(&name),
        );
        return Ok(());
    }
    let flat = join_path(module_dir, &format!("{name}.rs"))?;
    let nested = join_path(module_dir, &format!("{name}/mod.rs"))?;
    let matches = [flat, nested]
        .into_iter()
        .filter(|candidate| context.sources.contains_key(candidate))
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [selected] => context
            .queue
            .push_back((selected.clone(), join_path(module_dir, &name)?)),
        [] => boundary(
            context.boundaries,
            "RUST_MODULE_SOURCE_MISSING",
            Some(context.path),
            Some(&name),
        ),
        _ => boundary(
            context.boundaries,
            "RUST_MODULE_SOURCE_AMBIGUOUS",
            Some(context.path),
            Some(&name),
        ),
    }
    Ok(())
}

fn note_attributes(attrs: &[syn::Attribute], context: &mut SyntaxContext<'_>, subject: &str) {
    if has_attribute(attrs, "cfg") {
        boundary(
            context.boundaries,
            "RUST_CFG_NOT_EVALUATED",
            Some(context.path),
            Some(subject),
        );
    }
    if has_attribute(attrs, "cfg_attr") {
        boundary(
            context.boundaries,
            "RUST_CFG_ATTR_NOT_EVALUATED",
            Some(context.path),
            Some(subject),
        );
    }
    if has_attribute(attrs, "derive") {
        boundary(
            context.boundaries,
            "RUST_DERIVE_NOT_EXPANDED",
            Some(context.path),
            Some(subject),
        );
    }
    if attrs.iter().any(|attribute| {
        !BUILTIN_ATTRIBUTES
            .iter()
            .any(|name| attribute.path().is_ident(name))
    }) {
        boundary(
            context.boundaries,
            "RUST_ATTRIBUTE_MACRO_UNKNOWN",
            Some(context.path),
            Some(subject),
        );
    }
}

fn has_attribute(attrs: &[syn::Attribute], name: &str) -> bool {
    attrs
        .iter()
        .any(|attribute| attribute.path().is_ident(name))
}

fn has_any_attribute(attrs: &[syn::Attribute], names: &[&str]) -> bool {
    names.iter().any(|name| has_attribute(attrs, name))
}

const BUILTIN_ATTRIBUTES: &[&str] = &[
    "cfg",
    "cfg_attr",
    "derive",
    "doc",
    "allow",
    "warn",
    "deny",
    "forbid",
    "inline",
    "must_use",
    "deprecated",
    "repr",
    "non_exhaustive",
    "path",
    "test",
];

fn item_attrs(item: &syn::Item) -> &[syn::Attribute] {
    match item {
        syn::Item::ExternCrate(v) => &v.attrs,
        syn::Item::ForeignMod(v) => &v.attrs,
        syn::Item::Macro(v) => &v.attrs,
        syn::Item::TraitAlias(v) => &v.attrs,
        syn::Item::Union(v) => &v.attrs,
        syn::Item::Use(v) => &v.attrs,
        _ => &[],
    }
}

fn line_starts(source: &str) -> Vec<usize> {
    std::iter::once(0)
        .chain(
            source
                .bytes()
                .enumerate()
                .filter_map(|(index, byte)| (byte == b'\n').then_some(index + 1)),
        )
        .collect()
}

fn offset(
    source: &str,
    starts: &[usize],
    location: proc_macro2::LineColumn,
) -> Result<usize, ClewError> {
    let line = location
        .line
        .checked_sub(1)
        .ok_or_else(|| invalid("Rust parser returned an invalid line"))?;
    let start = *starts
        .get(line)
        .ok_or_else(|| invalid("Rust parser span escaped its source"))?;
    let offset = start.saturating_add(location.column);
    if offset > source.len() || !source.is_char_boundary(offset) {
        return Err(invalid("Rust parser span escaped its source"));
    }
    Ok(offset)
}

fn root_module_dir(source_path: &str) -> Result<String, ClewError> {
    let parent = Path::new(source_path)
        .parent()
        .unwrap_or_else(|| Path::new(""));
    relative_path(parent)
}

fn join_path(base: &str, suffix: &str) -> Result<String, ClewError> {
    relative_path(&PathBuf::from(base).join(suffix))
}

fn relative_path(path: &Path) -> Result<String, ClewError> {
    let text = path
        .to_str()
        .ok_or_else(|| invalid("Rust module path is not UTF-8"))?;
    if text.is_empty() {
        Ok(String::new())
    } else if safe_path(text) {
        Ok(text.replace('\\', "/"))
    } else {
        Err(invalid("Rust module path is outside the repository"))
    }
}

fn boundary(
    boundaries: &mut BTreeSet<String>,
    code: &str,
    file: Option<&str>,
    subject: Option<&str>,
) {
    let value = json!({"code":code,"file":file,"subject":subject,"resolution":"UNKNOWN"});
    let encoded =
        String::from_utf8(canonical::bytes(&value).expect("static boundary is canonical"))
            .expect("canonical JSON is UTF-8");
    if boundaries.contains(&encoded) {
        return;
    }
    if boundaries.len() < MAX_BOUNDARIES - 1 {
        boundaries.insert(encoded);
    } else {
        let limit = json!({
            "code":"RUST_BOUNDARY_LIMIT",
            "file":Value::Null,
            "subject":Value::Null,
            "resolution":"UNKNOWN"
        });
        boundaries.insert(
            String::from_utf8(canonical::bytes(&limit).expect("static boundary is canonical"))
                .expect("canonical JSON is UTF-8"),
        );
    }
}

fn safe_path(path: &str) -> bool {
    !path.is_empty()
        && !path.starts_with('/')
        && !path.contains('\0')
        && path
            .split('/')
            .all(|component| !component.is_empty() && !matches!(component, "." | ".."))
}

fn require_digest(value: &str) -> Result<(), ClewError> {
    if value.len() != 71
        || !value.starts_with("sha256:")
        || !value[7..]
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Err(invalid("Rust adapter authority digest is invalid"));
    }
    Ok(())
}

fn cancelled_error() -> ClewError {
    ClewError::new(
        ErrorCode::TransactionRecoveryRequired,
        "Rust analysis was cancelled",
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
    use super::{
        PreparedFactBatch, RUST_INDEX_SCHEMA, RustSyntaxAuthority, build_syntax_index,
        translate_facts,
    };
    use crate::cas::CasStore;
    use crate::repository_snapshot;
    use crate::state::StateAuthority;
    use serde_json::json;
    use std::fs;
    use std::process::Command;

    fn digest(character: char) -> String {
        format!("sha256:{}", character.to_string().repeat(64))
    }

    #[test]
    fn cargo_target_facts_are_granular_path_safe_and_deterministic() {
        let temporary = tempfile::tempdir().unwrap();
        let state = StateAuthority::open(temporary.path().join("v2")).unwrap();
        let store = CasStore::open(&state).unwrap();
        let index = json!({
            "schema":RUST_INDEX_SCHEMA,
            "compilation":"cargo-Cargo.toml-demo-lib-demo",
            "modelDigest":digest('1'),
            "package":"demo",
            "targetKind":"lib",
            "targetName":"demo",
            "sourcePath":"src/lib.rs",
            "cargoVersion":"cargo 1.92.0",
            "rustcVersion":"rustc 1.92.0",
            "parserAuthority":super::PARSER_AUTHORITY,
            "analysisCoverage":"PARTIAL",
            "analysisCertainty":"UNSURE",
            "files":[{"path":"src/lib.rs","contentHash":digest('2')}],
            "declarationDescriptors":{"coverage":"PARTIAL","descriptors":[]},
            "declarationRelations":{"coverage":"PARTIAL","relations":[]},
            "boundaries":[],
        });
        let first = translate_facts(&store, &index).unwrap();
        let second = translate_facts(&store, &index).unwrap();
        assert_eq!(first, second);
        assert_eq!(first.len(), 2);
        assert_eq!(
            regular_file_count(&temporary.path().join("v2/objects/sha256")),
            0
        );
        assert_eq!(
            fs::read_dir(temporary.path().join("v2/objects/packs-v3"))
                .unwrap()
                .filter_map(Result::ok)
                .filter(|entry| entry.path().extension() == Some(std::ffi::OsStr::new("pack")))
                .count(),
            1
        );
        for fact in first {
            let lease = store.read(&fact.payload, 4096).unwrap();
            let text = std::str::from_utf8(lease.bytes()).unwrap();
            assert!(!text.contains("/Users/"));
            assert!(!text.contains("file://"));
        }
    }

    #[test]
    fn fact_batch_input_limit_rejects_before_append() {
        let first = json!({"schema":"test/fact/1","name":"first"});
        let second = json!({"schema":"test/fact/1","name":"second"});
        let exact = crate::canonical::bytes(&first).unwrap().len();
        let mut batch = PreparedFactBatch::default();
        batch.push(first, exact).unwrap();
        assert_eq!(batch.input_bytes, exact);
        assert_eq!(batch.prepared.len(), 1);
        let retained = batch.prepared.clone();
        assert!(batch.push(second, exact).is_err());
        assert_eq!(batch.input_bytes, exact);
        assert_eq!(batch.prepared, retained);
    }

    #[test]
    fn syntax_index_follows_only_unambiguous_modules_and_emits_no_relations() {
        let temporary = tempfile::tempdir().unwrap();
        let repo = temporary.path().join("repo");
        fs::create_dir_all(repo.join("src")).unwrap();
        fs::write(
            repo.join("src/lib.rs"),
            "mod helper;\npub fn format_message() {}\nmacro_rules! generated { () => {} }\n",
        )
        .unwrap();
        fs::write(
            repo.join("src/helper.rs"),
            "#[derive(Debug)]\npub struct Formatter;\nimpl Formatter { pub fn format_message(&self) {} }\n#[cfg(test)] pub fn conditional() {}\n",
        )
        .unwrap();
        git(&repo, &["init", "-q"]);
        git(&repo, &["add", "."]);
        let state = StateAuthority::open(temporary.path().join("syntax-state-v2")).unwrap();
        let store = CasStore::open(&state).unwrap();
        let (snapshot, _) = repository_snapshot::capture(&repo, &store).unwrap();
        let first = build_syntax_index(&store, &snapshot, &syntax_authority(&digest('1'))).unwrap();
        let second =
            build_syntax_index(&store, &snapshot, &syntax_authority(&digest('1'))).unwrap();
        assert_eq!(first, second);
        assert_eq!(first["files"].as_array().unwrap().len(), 2);
        let descriptors = first["declarationDescriptors"]["descriptors"]
            .as_array()
            .unwrap();
        assert!(descriptors.iter().any(|row| {
            row["name"] == "format_message"
                && row["file"] == "src/lib.rs"
                && row["resolution"] == "SYNTAX_EXACT"
        }));
        assert!(
            descriptors
                .iter()
                .any(|row| { row["name"] == "format_message" && row["file"] == "src/helper.rs" })
        );
        assert!(
            first["declarationRelations"]["relations"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        let boundary_codes = first["boundaries"]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| row["code"].as_str().unwrap())
            .collect::<Vec<_>>();
        assert!(boundary_codes.contains(&"RUST_DERIVE_NOT_EXPANDED"));
        assert!(boundary_codes.contains(&"RUST_CFG_NOT_EVALUATED"));
        assert!(boundary_codes.contains(&"RUST_MACRO_ITEM_NOT_EXPANDED"));

        let loose_before =
            regular_file_count(&temporary.path().join("syntax-state-v2/objects/sha256"));
        let facts = translate_facts(&store, &first).unwrap();
        assert_eq!(
            regular_file_count(&temporary.path().join("syntax-state-v2/objects/sha256")),
            loose_before
        );
        assert!(facts.len() > 6);
        for fact in facts {
            let lease = store.read(&fact.payload, 64 * 1024).unwrap();
            assert!(lease.bytes().len() <= 64 * 1024);
            let text = std::str::from_utf8(lease.bytes()).unwrap();
            assert!(!text.contains("/Users/"));
            assert!(!text.contains("/private/"));
            assert!(!text.contains("\"target\":"));
        }
    }

    #[test]
    fn ambiguous_external_module_is_a_boundary_not_an_expansion() {
        let temporary = tempfile::tempdir().unwrap();
        let repo = temporary.path().join("repo");
        fs::create_dir_all(repo.join("src/helper")).unwrap();
        fs::write(repo.join("src/lib.rs"), "mod helper;\n").unwrap();
        fs::write(repo.join("src/helper.rs"), "pub fn flat() {}\n").unwrap();
        fs::write(repo.join("src/helper/mod.rs"), "pub fn nested() {}\n").unwrap();
        git(&repo, &["init", "-q"]);
        git(&repo, &["add", "."]);
        let state = StateAuthority::open(temporary.path().join("ambiguity-state-v2")).unwrap();
        let store = CasStore::open(&state).unwrap();
        let (snapshot, _) = repository_snapshot::capture(&repo, &store).unwrap();
        let index = build_syntax_index(&store, &snapshot, &syntax_authority(&digest('1'))).unwrap();
        assert_eq!(index["files"].as_array().unwrap().len(), 1);
        assert!(
            index["boundaries"]
                .as_array()
                .unwrap()
                .iter()
                .any(|row| { row["code"] == "RUST_MODULE_SOURCE_AMBIGUOUS" })
        );
        assert!(
            index["declarationRelations"]["relations"]
                .as_array()
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn cfg_attr_module_and_namespaced_builtin_names_fail_closed() {
        let temporary = tempfile::tempdir().unwrap();
        let repo = temporary.path().join("repo");
        fs::create_dir_all(repo.join("src")).unwrap();
        fs::write(
            repo.join("src/lib.rs"),
            "#[cfg_attr(feature = \"alternate\", path = \"alternate.rs\")]\nmod selected;\n#[my::cfg]\npub fn namespaced_attribute() {}\n",
        )
        .unwrap();
        fs::write(repo.join("src/selected.rs"), "pub fn default_child() {}\n").unwrap();
        fs::write(
            repo.join("src/alternate.rs"),
            "pub fn alternate_child() {}\n",
        )
        .unwrap();
        git(&repo, &["init", "-q"]);
        git(&repo, &["add", "."]);
        let state = StateAuthority::open(temporary.path().join("cfg-state-v2")).unwrap();
        let store = CasStore::open(&state).unwrap();
        let (snapshot, _) = repository_snapshot::capture(&repo, &store).unwrap();
        let index = build_syntax_index(&store, &snapshot, &syntax_authority(&digest('1'))).unwrap();
        assert_eq!(index["files"].as_array().unwrap().len(), 1);
        let names = index["declarationDescriptors"]["descriptors"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|row| row["name"].as_str())
            .collect::<Vec<_>>();
        assert!(!names.contains(&"default_child"));
        assert!(!names.contains(&"alternate_child"));
        assert!(
            index["declarationDescriptors"]["descriptors"]
                .as_array()
                .unwrap()
                .iter()
                .any(|row| row["name"] == "selected" && row["cfgStatus"] == "UNKNOWN")
        );
        let boundaries = index["boundaries"].as_array().unwrap();
        assert!(
            boundaries
                .iter()
                .any(|row| { row["code"] == "RUST_CFG_ATTR_MODULE_NOT_EXPANDED" })
        );
        assert!(
            boundaries
                .iter()
                .any(|row| { row["code"] == "RUST_CFG_ATTR_NOT_EVALUATED" })
        );
        assert!(boundaries.iter().any(|row| {
            row["code"] == "RUST_ATTRIBUTE_MACRO_UNKNOWN"
                && row["subject"] == "namespaced_attribute"
        }));
        assert!(
            index["declarationRelations"]["relations"]
                .as_array()
                .unwrap()
                .is_empty()
        );
    }

    fn git(repo: &std::path::Path, arguments: &[&str]) {
        assert!(
            Command::new("git")
                .args(arguments)
                .current_dir(repo)
                .status()
                .unwrap()
                .success()
        );
    }

    fn syntax_authority(model_digest: &str) -> RustSyntaxAuthority<'_> {
        RustSyntaxAuthority {
            compilation_id: "cargo-Cargo.toml-demo-lib-demo",
            model_digest,
            package: "demo",
            target_kind: "lib",
            target_name: "demo",
            source_path: "src/lib.rs",
            cargo_version: "cargo 1.92.0",
            rustc_version: "rustc 1.92.0",
        }
    }

    fn regular_file_count(root: &std::path::Path) -> usize {
        fs::read_dir(root)
            .unwrap()
            .filter_map(Result::ok)
            .map(|entry| {
                if entry.file_type().unwrap().is_dir() {
                    regular_file_count(&entry.path())
                } else {
                    usize::from(entry.file_type().unwrap().is_file())
                }
            })
            .sum()
    }
}
