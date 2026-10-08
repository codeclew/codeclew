//! Bounded declared OpenAPI producer. Never executes code or fetches references.
use super::{analysis, digest, invalid, model::*};
use crate::{
    canonical,
    cas::CasStore,
    error::ClewError,
    repository_snapshot::{TrackedScopeLimits, capture_documentation_scope},
    state::StateAuthority,
};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Component, Path},
};

pub const SCHEMA: &str = "codeclew-documentation-contract-facts/1.0";
pub const TESTED_VERSIONS: &[&str] = &[
    "3.0.0", "3.0.3", "3.0.4", "3.0.123", "3.1.0", "3.1.1", "3.1.2", "3.1.123", "3.2.0", "3.99.7",
    "4.0.0",
];
pub const SUPPORTED_VERSION_RANGE: &str = "OPENAPI_THROUGH_3_X";
pub const READER: &str = "BOUNDED_OPENAPI_PATH_OPERATION_READER";
pub const COMPATIBILITY_ANCHORS: &[&str] = &["3.0", "3.1"];
const METHODS: &[&str] = &[
    "get", "put", "post", "delete", "options", "head", "patch", "trace",
];
pub const LIMITATIONS: &[&str] = &[
    "DECLARATIONS_NOT_RUNTIME_ENFORCEMENT",
    "NO_IMPLICIT_NETWORK",
    "NO_SCHEMA_INSTANCE_VALIDATION",
    "CALLBACKS_RETAINED_NOT_SOURCE_MAPPED",
    "WEBHOOKS_RETAINED_NOT_SOURCE_MAPPED",
    "JSON_SCHEMA_DIALECT_AND_2020_12_SEMANTICS_NOT_INTERPRETED",
    "SCHEMA_NULL_AND_UNION_TYPES_RETAINED_NOT_INTERPRETED",
    "UNKNOWN_PATH_ITEM_FIELDS_RETAINED_NOT_INTERPRETED",
];
const MAX_FILE: usize = 2 * 1024 * 1024;

fn readable_path_item(item: &Value) -> bool {
    item.is_object()
        && item.get("$ref").is_none_or(Value::is_string)
        && METHODS
            .iter()
            .all(|method| item.get(*method).is_none_or(Value::is_object))
}

fn path_item_boundaries(item: &Value) -> BTreeSet<String> {
    let mut gaps = BTreeSet::new();
    if item.as_object().is_some_and(|map| {
        map.keys().any(|key| {
            !key.starts_with("x-")
                && !METHODS.contains(&key.as_str())
                && !matches!(
                    key.as_str(),
                    "$ref" | "summary" | "description" | "servers" | "parameters"
                )
        })
    }) {
        gaps.insert("OPENAPI_UNKNOWN_PATH_ITEM_FIELDS_RETAINED_NOT_INTERPRETED".into());
    }
    gaps
}

/// Every version attempts the same structural reader. Anchors describe compatibility assumptions,
/// not separate parsers or full specification validation; only unreadable shapes are rejected.
fn select_reader(document: &Value) -> Result<Option<Value>, &'static str> {
    if document.get("openapi").is_none() && document.get("paths").is_none() {
        return Ok(None); // Registered reference fragments may have any JSON root shape.
    }
    let root = document.as_object().ok_or("UNREADABLE_CONTRACT_ROOT")?;
    if let Some(paths) = root.get("paths") {
        let paths = paths.as_object().ok_or("UNREADABLE_CONTRACT_PATHS")?;
        if paths.iter().any(|(path, item)| {
            !path.starts_with("x-") && (!path.starts_with('/') || !readable_path_item(item))
        }) {
            return Err("UNREADABLE_CONTRACT_PATHS");
        }
    }
    let version = root.get("openapi");
    let parts: Vec<_> = version
        .and_then(Value::as_str)
        .unwrap_or("")
        .split('.')
        .collect();
    let hint = parts
        .first()
        .and_then(|p| p.parse::<u64>().ok())
        .zip(parts.get(1).and_then(|p| p.parse::<u64>().ok()));
    let well_formed = parts.len() == 3
        && parts.iter().all(|p| {
            !p.is_empty()
                && (*p == "0" || !p.starts_with('0'))
                && p.bytes().all(|b| b.is_ascii_digit())
        })
        && hint.is_some();
    let anchor = match hint {
        Some((major, minor)) if major < 3 || (major == 3 && minor == 0) => "3.0",
        _ => "3.1",
    };
    let mode = if version.is_none() {
        "MISSING_VERSION_ASSUMED"
    } else if !well_formed {
        "MALFORMED_VERSION_ASSUMED"
    } else if hint.is_some_and(|(major, minor)| major == 3 && minor <= 1) {
        "DECLARED_COMPATIBILITY_PROFILE"
    } else {
        "NEAREST_COMPATIBILITY_PROFILE"
    };
    Ok(Some(
        json!({"reader":READER,"compatibilityAnchor":anchor,"selectionMode":mode,"supportedVersionRange":SUPPORTED_VERSION_RANGE,"outsideAdvertisedRange":hint.is_some_and(|(major, _)| major > 3),"fullSpecificationValidation":false}),
    ))
}

fn selection_boundaries(selection: &Value) -> BTreeSet<String> {
    let mut gaps = BTreeSet::new();
    match selection["selectionMode"].as_str() {
        Some("MISSING_VERSION_ASSUMED") => {
            gaps.insert("OPENAPI_MISSING_VERSION_ASSUMED".into());
        }
        Some("MALFORMED_VERSION_ASSUMED") => {
            gaps.insert("OPENAPI_MALFORMED_VERSION_ASSUMED".into());
        }
        Some("NEAREST_COMPATIBILITY_PROFILE") => {
            gaps.insert("OPENAPI_NEAREST_COMPATIBILITY_PROFILE".into());
        }
        _ => (),
    }
    if selection["outsideAdvertisedRange"] == true {
        gaps.insert("OPENAPI_OUTSIDE_ADVERTISED_RANGE_BEST_EFFORT".into());
    }
    gaps
}

fn reported_version(document: &Value, selection: &Value) -> Value {
    if matches!(
        selection["selectionMode"].as_str(),
        Some("DECLARED_COMPATIBILITY_PROFILE" | "NEAREST_COMPATIBILITY_PROFILE")
    ) {
        document.get("openapi").cloned().unwrap_or(Value::Null)
    } else {
        Value::Null
    }
}

/// Identify retained features we do not interpret without copying contract values into diagnostics.
/// Schema traversal excludes instance examples/defaults and property names are never keywords.
fn feature_boundaries(document: &Value) -> BTreeSet<String> {
    fn visit(
        value: &Value,
        schema: bool,
        depth: usize,
        remaining: &mut usize,
        gaps: &mut BTreeSet<String>,
    ) {
        if *remaining == 0 || depth > 32 {
            gaps.insert("CONTRACT_FEATURE_INSPECTION_LIMIT".into());
            return;
        }
        *remaining -= 1;
        if schema && value.is_boolean() {
            gaps.insert("OPENAPI_BOOLEAN_SCHEMA_NOT_INTERPRETED".into());
        }
        match value {
            Value::Object(map) => {
                if schema {
                    if map.contains_key("$schema") {
                        gaps.insert("OPENAPI_SCHEMA_DIALECT_NOT_INTERPRETED".into());
                    }
                    if map.get("type").is_some_and(|v| v.is_null() || v == "null") {
                        gaps.insert("OPENAPI_SCHEMA_NULL_TYPE_NOT_INTERPRETED".into());
                    }
                    if let Some(types) = map.get("type").and_then(Value::as_array) {
                        gaps.insert("OPENAPI_SCHEMA_TYPE_UNIONS_NOT_INTERPRETED".into());
                        if types.iter().any(|v| v.is_null() || v == "null") {
                            gaps.insert("OPENAPI_SCHEMA_NULL_TYPE_NOT_INTERPRETED".into());
                        }
                    }
                    if [
                        "$id",
                        "$anchor",
                        "$dynamicAnchor",
                        "$dynamicRef",
                        "$vocabulary",
                        "$defs",
                        "prefixItems",
                        "unevaluatedProperties",
                        "unevaluatedItems",
                        "dependentSchemas",
                        "dependentRequired",
                        "patternProperties",
                        "propertyNames",
                        "contains",
                        "minContains",
                        "maxContains",
                        "if",
                        "then",
                        "else",
                        "const",
                    ]
                    .iter()
                    .any(|k| map.contains_key(*k))
                    {
                        gaps.insert("OPENAPI_JSON_SCHEMA_KEYWORDS_NOT_INTERPRETED".into());
                    }
                }
                for (key, child) in map {
                    if schema {
                        match key.as_str() {
                            "properties" | "patternProperties" | "$defs" | "definitions"
                            | "dependentSchemas" => {
                                for nested in child.as_object().into_iter().flat_map(|m| m.values())
                                {
                                    visit(nested, true, depth + 1, remaining, gaps);
                                }
                            }
                            // Boolean additionalProperties is also valid in OpenAPI 3.0.
                            "additionalProperties" if child.is_boolean() => (),
                            "items"
                            | "additionalProperties"
                            | "unevaluatedProperties"
                            | "unevaluatedItems"
                            | "propertyNames"
                            | "contains"
                            | "not"
                            | "if"
                            | "then"
                            | "else"
                            | "allOf"
                            | "anyOf"
                            | "oneOf"
                            | "prefixItems" => visit(child, true, depth + 1, remaining, gaps),
                            _ => (),
                        }
                    } else if !matches!(
                        key.as_str(),
                        "example" | "examples" | "default" | "enum" | "const"
                    ) && !key.starts_with("x-")
                    {
                        if key == "components" {
                            for nested in child["schemas"]
                                .as_object()
                                .into_iter()
                                .flat_map(|m| m.values())
                            {
                                visit(nested, true, depth + 1, remaining, gaps);
                            }
                        }
                        visit(child, key == "schema", depth + 1, remaining, gaps);
                    }
                }
            }
            Value::Array(values) => {
                for child in values {
                    visit(child, schema, depth + 1, remaining, gaps);
                }
            }
            _ => (),
        }
    }
    let mut gaps = BTreeSet::new();
    for (path, item) in document["paths"].as_object().into_iter().flatten() {
        if path.starts_with('/') {
            gaps.extend(path_item_boundaries(item));
        }
    }
    if document.get("webhooks").is_some() {
        gaps.insert("OPENAPI_WEBHOOKS_NOT_SOURCE_MAPPED".into());
    }
    if document.get("jsonSchemaDialect").is_some() {
        gaps.insert("OPENAPI_SCHEMA_DIALECT_NOT_INTERPRETED".into());
    }
    visit(document, false, 0, &mut 100_000, &mut gaps);
    gaps
}

/// Contracts have their own committed selection, independent of language roots.
pub fn capture(
    service: &Service,
    repo: &Path,
    evidence: &mut ServiceEvidence,
) -> Result<(), ClewError> {
    if service.contract_files.is_empty() {
        return Ok(());
    }
    let state = StateAuthority::process_default()?;
    let cas = CasStore::open(&state)?;
    let (snapshot, _) = capture_documentation_scope(
        repo,
        &evidence.revision,
        &service.contract_files,
        &cas,
        TrackedScopeLimits {
            max_files: 128,
            max_file_bytes: MAX_FILE,
            max_total_bytes: 16 * 1024 * 1024,
            max_tree_entries: 100_000,
            max_tree_bytes: 16 * 1024 * 1024,
            max_tree_path_bytes: 4096,
        },
    )?;
    let mut files = BTreeMap::new();
    for entry in &snapshot.index {
        if !service.contract_files.contains(&entry.path) {
            continue;
        }
        if !matches!(entry.mode, 0o100644 | 0o100755) {
            evidence
                .boundaries
                .push(format!("UNSAFE_CONTRACT_FILE:{}", entry.path));
            continue;
        }
        let bytes = cas.read(&entry.content, MAX_FILE)?;
        match std::str::from_utf8(bytes.bytes()) {
            Ok(text) => {
                files.insert(entry.path.clone(), text.to_owned());
            }
            Err(_) => evidence
                .boundaries
                .push(format!("NON_UTF8_CONTRACT:{}", entry.path)),
        }
    }
    import(service, evidence, &files)?;
    for entry in &snapshot.index {
        let id = analysis::source_id(&service.id, &format!("contract/{}", entry.path))?;
        if let Some(source) = evidence.sources.get_mut(&id) {
            source.evidence_digest = digest(&(
                SOURCE_EXTRACTOR,
                &snapshot.snapshot_id,
                &entry.git_oid,
                0usize,
                source.text.len(),
            ))?;
            source.occurrence = Some(SourceOccurrence {
                snapshot: snapshot.snapshot_id.clone(),
                blob: entry.git_oid.clone(),
                start_byte: 0,
                end_byte: source.text.len(),
            });
        }
    }
    enrich(evidence)
}

/// Also used by the sealed JVM projection with already admitted file contents.
pub fn import(
    service: &Service,
    evidence: &mut ServiceEvidence,
    files: &BTreeMap<String, String>,
) -> Result<(), ClewError> {
    evidence.contracts.clear();
    let mut inventory = BTreeMap::new();
    for path in &service.contract_files {
        let Some(text) = files.get(path).filter(|t| !t.is_empty()) else {
            inventory.insert(
                path.clone(),
                json!({"status":"CONTRACT_SOURCE_UNAVAILABLE"}),
            );
            evidence
                .boundaries
                .push(format!("CONTRACT_SOURCE_UNAVAILABLE:{path}"));
            continue;
        };
        let hash = canonical::hash_bytes(text.as_bytes());
        let id = analysis::source_id(&service.id, &format!("contract/{path}"))?;
        evidence.sources.insert(
            id.clone(),
            Source {
                id: id.clone(),
                service: service.id.clone(),
                revision: evidence.revision.clone(),
                file: path.clone(),
                start_line: 1,
                end_line: text.lines().count() as u64,
                text: text.clone(),
                text_digest: hash.clone(),
                evidence_digest: hash.clone(),
                authority: "DECLARED_OPENAPI".into(),
                occurrence: None,
                url: analysis::source_link(
                    service,
                    &evidence.revision,
                    path,
                    1,
                    text.lines().count() as u64,
                ),
            },
        );
        let value: Value = match serde_yaml_ng::from_str(text) {
            Ok(value) => value,
            Err(_) => {
                inventory.insert(
                    path.clone(),
                    json!({"digest":hash,"status":"INVALID_CONTRACT_JSON_YAML"}),
                );
                evidence
                    .boundaries
                    .push(format!("INVALID_CONTRACT_JSON_YAML:{path}"));
                continue;
            }
        };
        let admission = select_reader(&value);
        let readable = admission.is_ok();
        let (status, selection) = match admission {
            Ok(Some(selection)) => ("DECLARED_OPENAPI", selection),
            Ok(None) => ("REGISTERED_REFERENCE_DOCUMENT", Value::Null),
            Err(code) => {
                evidence.boundaries.push(code.into());
                (code, Value::Null)
            }
        };
        let mut features = if readable {
            feature_boundaries(&value)
        } else {
            BTreeSet::new()
        };
        features.extend(selection_boundaries(&selection));
        if readable && !selection.is_null() && value.get("paths").is_none() {
            features.insert("OPENAPI_NO_PATH_OPERATIONS".into());
        }
        evidence.boundaries.extend(features.iter().cloned());
        let version = reported_version(&value, &selection);
        inventory.insert(
            path.clone(),
            json!({"digest":hash,"version":version,"status":status,"readerSelection":selection,"boundaries":features}),
        );
        let dep = analysis::dependency_id(&service.id, "contract", path)?;
        evidence.observations.insert(
            dep.clone(),
            Observation {
                id: dep,
                kind: "CONTRACT".into(),
                service: service.id.clone(),
                symbol: path.clone(),
                digest: digest(&value)?,
                normalized: value.clone(),
                source_ids: vec![id],
            },
        );
        // Unreadable shapes remain retained declarations, never interpreted as path operations.
        if readable {
            evidence.contracts.insert(path.clone(), value);
        }
    }
    let normalized = json!({"schema":SCHEMA,"authority":"DECLARED_OPENAPI","inputs":inventory,"implementationDigest":canonical::hash_bytes(include_bytes!("contracts.rs")),"supportedVersionRange":SUPPORTED_VERSION_RANGE,"reader":READER,"compatibilityAnchors":COMPATIBILITY_ANCHORS,"testedVersions":TESTED_VERSIONS,"compatibilityScope":"RETAINED_PATH_OPERATIONS_AND_BOUNDED_REFERENCES","limitations":LIMITATIONS});
    let id = analysis::dependency_id(&service.id, "contract-scope", "selected-contracts")?;
    evidence.observations.insert(
        id.clone(),
        Observation {
            id,
            kind: "CONTRACT_SCOPE".into(),
            service: service.id.clone(),
            symbol: "selected-contracts".into(),
            digest: digest(&normalized)?,
            normalized,
            source_ids: vec![],
        },
    );
    Ok(())
}

struct Resolver<'a> {
    documents: &'a BTreeMap<String, Value>,
    seen: BTreeSet<String>,
    gaps: BTreeSet<String>,
    files: BTreeSet<String>,
    remaining: usize,
}
impl Resolver<'_> {
    fn resolve(&mut self, value: &Value, file: &str, depth: usize) -> Result<Value, ClewError> {
        if self.remaining == 0 {
            return Err(invalid(
                "contract reference expansion exceeds 100000 values; narrow selected contracts",
            ));
        }
        self.remaining -= 1;
        if depth > 32 {
            self.gaps.insert("CONTRACT_REFERENCE_DEPTH_EXCEEDED".into());
            return Ok(value.clone());
        }
        match value {
            Value::Object(map) => {
                if let Some(reference) = map.get("$ref").and_then(Value::as_str) {
                    if map.len() > 1 {
                        self.gaps.insert(format!(
                            "REFERENCE_SIBLINGS_NOT_INTERPRETED:{file}:{reference}"
                        ));
                    }
                    let (relative, fragment) = reference.split_once('#').unwrap_or((reference, ""));
                    let mut target = Path::new(file)
                        .parent()
                        .unwrap_or(Path::new(""))
                        .to_path_buf();
                    if relative.is_empty() {
                        target = Path::new(file).to_path_buf();
                    } else {
                        if relative.contains([':', '\\', '?', '%']) || relative.starts_with('/') {
                            self.gaps
                                .insert(format!("EXTERNAL_CONTRACT_REFERENCE:{reference}"));
                            return Ok(value.clone());
                        }
                        for component in Path::new(relative).components() {
                            match component {
                                Component::Normal(p) => target.push(p),
                                Component::CurDir => (),
                                Component::ParentDir if target.pop() => (),
                                _ => {
                                    self.gaps
                                        .insert(format!("UNSAFE_CONTRACT_REFERENCE:{reference}"));
                                    return Ok(value.clone());
                                }
                            }
                        }
                    }
                    let target = target.to_string_lossy().to_string();
                    let Some(document) = self.documents.get(&target) else {
                        self.gaps.insert(format!(
                            "UNREGISTERED_OR_UNAVAILABLE_CONTRACT_REFERENCE:{target}"
                        ));
                        return Ok(value.clone());
                    };
                    self.files.insert(target.clone());
                    let key = format!("{target}#{fragment}");
                    if !self.seen.insert(key.clone()) {
                        self.gaps.insert(format!("CYCLIC_CONTRACT_REFERENCE:{key}"));
                        return Ok(value.clone());
                    }
                    let out = match document.pointer(fragment) {
                        Some(v) => self.resolve(v, &target, depth + 1)?,
                        None => {
                            self.gaps
                                .insert(format!("MISSING_CONTRACT_REFERENCE:{key}"));
                            value.clone()
                        }
                    };
                    self.seen.remove(&key);
                    return Ok(out);
                }
                map.iter()
                    .map(|(k, v)| Ok((k.clone(), self.resolve(v, file, depth + 1)?)))
                    .collect::<Result<serde_json::Map<String, Value>, ClewError>>()
                    .map(Value::Object)
            }
            Value::Array(a) => a
                .iter()
                .map(|v| self.resolve(v, file, depth + 1))
                .collect::<Result<Vec<_>, _>>()
                .map(Value::Array),
            _ => Ok(value.clone()),
        }
    }
}

pub fn enrich(evidence: &mut ServiceEvidence) -> Result<(), ClewError> {
    evidence
        .observations
        .retain(|_, o| o.kind != "CONTRACT_OPERATION");
    let mut budget = 100_000;
    let mut contract_features = BTreeSet::new();
    for (file, document) in &evidence.contracts {
        let selection = match select_reader(document) {
            Ok(Some(selection)) => selection,
            Ok(None) => continue,
            Err(code) => {
                contract_features.insert(code.into());
                continue;
            }
        };
        contract_features.extend(feature_boundaries(document));
        if document.get("paths").is_none() {
            contract_features.insert("OPENAPI_NO_PATH_OPERATIONS".into());
        }
        let selection_gaps = selection_boundaries(&selection);
        contract_features.extend(selection_gaps.iter().cloned());
        for (path, item) in document["paths"].as_object().into_iter().flatten() {
            if path.starts_with("x-") {
                continue;
            }
            let mut resolver = Resolver {
                documents: &evidence.contracts,
                seen: BTreeSet::new(),
                gaps: selection_gaps.clone(),
                files: BTreeSet::from([file.clone()]),
                remaining: budget,
            };
            let item = resolver.resolve(item, file, 0)?;
            if !readable_path_item(&item) {
                contract_features.insert("UNREADABLE_RESOLVED_CONTRACT_PATH_ITEM".into());
                budget = resolver.remaining;
                continue;
            }
            let item_gaps = path_item_boundaries(&item);
            contract_features.extend(item_gaps.iter().cloned());
            resolver.gaps.extend(item_gaps);
            for method in METHODS.iter().copied() {
                let Some(operation) = item.get(method).filter(|v| v.is_object()) else {
                    continue;
                };
                let mut parameters = BTreeMap::new();
                for p in item["parameters"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .chain(operation["parameters"].as_array().into_iter().flatten())
                {
                    parameters.insert(format!("{}:{}", p["in"], p["name"]), p.clone());
                }
                let matched: Vec<_> = evidence
                    .entrypoints
                    .iter()
                    .filter(|e| {
                        e.kind == "HTTP_ENDPOINT"
                            && e.trigger["paths"]
                                .as_array()
                                .is_some_and(|a| a.iter().any(|p| p == path))
                            && e.trigger["methods"].as_array().is_some_and(|a| {
                                a.iter().any(|m| {
                                    m.as_str().is_some_and(|m| m.eq_ignore_ascii_case(method))
                                })
                            })
                    })
                    .collect();
                let mapping = match matched.len() {
                    0 => "NO_SOURCE_ROUTE_MATCH",
                    1 => "SOURCE_ROUTE_MATCH_ONLY",
                    _ => "AMBIGUOUS_SOURCE_ROUTE_MATCH",
                };
                let entry = (matched.len() == 1).then(|| matched[0]);
                let security = resolver.resolve(
                    operation.get("security").unwrap_or(&document["security"]),
                    file,
                    0,
                )?;
                let schemes =
                    resolver.resolve(&document["components"]["securitySchemes"], file, 0)?;
                let servers = resolver.resolve(
                    operation
                        .get("servers")
                        .or_else(|| item.get("servers"))
                        .unwrap_or(&document["servers"]),
                    file,
                    0,
                )?;
                let mut normalized = json!({"schema":SCHEMA,"entrypoint":entry.map(|e|&e.id),"sourceMapping":mapping,"method":method.to_uppercase(),"path":path,"operation":operation,"parameters":parameters.values().collect::<Vec<_>>(),"security":security,"securitySchemes":schemes,"servers":servers,"declaredSource":file,"openapi":reported_version(document, &selection),"readerSelection":selection,"authority":"DECLARED_OPENAPI","runtimeEnforcement":"UNVERIFIED"});
                let operation_features = feature_boundaries(&normalized);
                contract_features.extend(operation_features.iter().cloned());
                resolver.gaps.extend(operation_features);
                if document.get("jsonSchemaDialect").is_some() {
                    resolver
                        .gaps
                        .insert("OPENAPI_SCHEMA_DIALECT_NOT_INTERPRETED".into());
                }
                normalized["boundaries"] = json!(resolver.gaps);
                let id = analysis::dependency_id(
                    &evidence.service,
                    "contract-operation",
                    &format!("{file}:{method}:{path}"),
                )?;
                let source_ids = resolver
                    .files
                    .iter()
                    .map(|file| analysis::source_id(&evidence.service, &format!("contract/{file}")))
                    .collect::<Result<Vec<_>, _>>()?;
                if source_ids
                    .iter()
                    .any(|id| !evidence.sources.contains_key(id))
                {
                    return Err(invalid("declared contract source is unavailable"));
                }
                evidence.observations.insert(
                    id.clone(),
                    Observation {
                        id,
                        kind: "CONTRACT_OPERATION".into(),
                        service: evidence.service.clone(),
                        symbol: entry
                            .map(|e| e.symbol.clone())
                            .unwrap_or_else(|| format!("{method} {path}")),
                        digest: digest(&normalized)?,
                        normalized,
                        source_ids,
                    },
                );
            }
            budget = resolver.remaining;
        }
    }
    evidence
        .boundaries
        .extend(contract_features.iter().cloned());
    if let Some(scope) = evidence
        .observations
        .values_mut()
        .find(|o| o.kind == "CONTRACT_SCOPE")
    {
        scope.normalized["boundaries"] = json!(contract_features);
        scope.digest = digest(&scope.normalized)?;
    }
    evidence.boundaries.sort();
    evidence.boundaries.dedup();
    Ok(())
}

pub fn for_entry<'a>(evidence: &'a ServiceEvidence, entry: &str) -> Vec<&'a Observation> {
    evidence
        .observations
        .values()
        .filter(|o| o.kind == "CONTRACT_OPERATION" && o.normalized["entrypoint"] == entry)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn imported_contract(version: &str) -> ServiceEvidence {
        imported_contract_text(
            &include_str!("../../../../fixtures/documentation-system/openapi/api.yaml")
                .replace("3.0.3", version),
        )
    }

    fn imported_contract_text(text: &str) -> ServiceEvidence {
        imported_contract_with_reference(
            text,
            include_str!("../../../../fixtures/documentation-system/openapi/types.yaml"),
        )
    }

    fn imported_contract_with_reference(text: &str, reference: &str) -> ServiceEvidence {
        let service: Service = serde_json::from_value(json!({
            "schema":"codeclew-documentation-service/1.0", "id":"orders",
            "title":"Orders", "repositoryId":"orders", "repository":"https://example.invalid/orders",
            "language":"java", "profile":"source-syntax", "targetRef":"main",
            "contractFiles":["api/api.yaml", "api/types.yaml"]
        })).unwrap();
        let mut evidence = ServiceEvidence {
            schema: "codeclew-documentation-service-evidence/1.0".into(),
            service: service.id.clone(),
            revision: "a".repeat(40),
            service_digest: String::new(),
            extractor: SOURCE_EXTRACTOR.into(),
            runtime_mode: "COMMITTED_SOURCE_NO_BUILD".into(),
            coverage: "PARTIAL".into(),
            boundaries: vec![],
            entrypoints: vec![],
            observations: BTreeMap::new(),
            sources: BTreeMap::new(),
            contracts: BTreeMap::new(),
        };
        let files = BTreeMap::from([
            ("api/api.yaml".into(), text.into()),
            ("api/types.yaml".into(), reference.into()),
        ]);
        import(&service, &mut evidence, &files).unwrap();
        evidence
    }

    #[test]
    fn declared_versions_choose_nearest_reader_and_attempt_path_operations() {
        for &version in TESTED_VERSIONS {
            let mut evidence = imported_contract(version);
            assert!(
                evidence.contracts.contains_key("api/api.yaml"),
                "{version}: {:?}",
                evidence.boundaries
            );
            enrich(&mut evidence).unwrap();
            let operations: Vec<_> = evidence
                .observations
                .values()
                .filter(|o| o.kind == "CONTRACT_OPERATION")
                .collect();
            assert_eq!(operations.len(), 2, "{version}");
            let post = operations
                .iter()
                .find(|o| o.normalized["method"] == "POST")
                .unwrap();
            assert_eq!(post.normalized["openapi"], version);
            assert_eq!(post.normalized["readerSelection"]["reader"], READER);
            assert_eq!(
                post.normalized["readerSelection"]["compatibilityAnchor"],
                if version.starts_with("3.0.") {
                    "3.0"
                } else {
                    "3.1"
                }
            );
            assert_eq!(
                post.normalized["readerSelection"]["outsideAdvertisedRange"],
                version.starts_with("4.")
            );
            assert_eq!(post.source_ids.len(), 2);
            assert_eq!(post.normalized["parameters"][0]["required"], false);
            assert_eq!(
                post.normalized["operation"]["requestBody"]["content"]["application/json"]["schema"]
                    ["properties"]["lines"]["items"]["properties"]["quantity"]["maximum"],
                100
            );
            assert_eq!(
                post.normalized["securitySchemes"]["token"]["scheme"],
                "bearer"
            );
            assert_eq!(
                post.normalized["servers"][0]["url"],
                "https://example.invalid"
            );
        }
    }

    #[test]
    fn compatibility_profile_selection_is_observable_and_not_an_admission_gate() {
        for (version, anchor, mode, outside) in [
            ("3.0.4", "3.0", "DECLARED_COMPATIBILITY_PROFILE", false),
            ("3.1.123", "3.1", "DECLARED_COMPATIBILITY_PROFILE", false),
            ("3.2.0", "3.1", "NEAREST_COMPATIBILITY_PROFILE", false),
            ("3.99.7", "3.1", "NEAREST_COMPATIBILITY_PROFILE", false),
            ("4.0.0", "3.1", "NEAREST_COMPATIBILITY_PROFILE", true),
            ("2.0.0", "3.0", "NEAREST_COMPATIBILITY_PROFILE", false),
            ("3.0.bad", "3.0", "MALFORMED_VERSION_ASSUMED", false),
            (
                "private-version-value",
                "3.1",
                "MALFORMED_VERSION_ASSUMED",
                false,
            ),
        ] {
            let selection = select_reader(&json!({"openapi":version,"paths":{}}))
                .unwrap()
                .unwrap();
            assert_eq!(selection["compatibilityAnchor"], anchor);
            assert_eq!(selection["selectionMode"], mode);
            assert_eq!(selection["outsideAdvertisedRange"], outside);
            assert!(!selection.to_string().contains("private-version-value"));
        }
    }

    #[test]
    fn openapi_31_path_item_references_and_inherited_parameters_are_retained() {
        let text = json!({
            "openapi":"3.1.2", "info":{"title":"Orders","version":"1"},
            "security":[{"token":[]}], "servers":[{"url":"https://example.invalid"}],
            "paths":{"/orders":{"$ref":"#/components/pathItems/Orders"}},
            "components":{
                "pathItems":{"Orders":{
                    "parameters":[{"$ref":"#/components/parameters/Trace"}],
                    "post":{"parameters":[{"name":"trace","in":"header","required":false,"schema":{"type":"string"}}],"responses":{"200":{"description":"OK","content":{"application/json":{"schema":{"$ref":"types.yaml#/Order"}}}}}}
                }},
                "parameters":{"Trace":{"name":"trace","in":"header","required":true,"schema":{"type":"string"}}},
                "securitySchemes":{"token":{"type":"http","scheme":"bearer"}}
            }
        }).to_string();
        let mut evidence = imported_contract_text(&text);
        enrich(&mut evidence).unwrap();
        let operation = evidence
            .observations
            .values()
            .find(|o| o.kind == "CONTRACT_OPERATION")
            .unwrap();
        assert_eq!(operation.normalized["path"], "/orders");
        assert_eq!(operation.normalized["method"], "POST");
        assert_eq!(operation.normalized["parameters"][0]["required"], false);
        assert_eq!(
            operation.normalized["operation"]["responses"]["200"]["content"]["application/json"]["schema"]
                ["properties"]["lines"]["minItems"],
            1
        );
        assert_eq!(operation.source_ids.len(), 2);
        assert_eq!(operation.normalized["security"], json!([{"token":[]}]));
        assert_eq!(
            operation.normalized["servers"][0]["url"],
            "https://example.invalid"
        );
    }

    #[test]
    fn missing_or_malformed_versions_attempt_readable_paths_with_explicit_assumptions() {
        for version in [
            json!("3.1"),
            json!("3.1.02"),
            json!("3.1.2-beta"),
            json!("private-version-value"),
            json!(3.1),
            Value::Null,
        ] {
            let text = json!({"openapi":version,"paths":{"/orders":{"get":{"responses":{"200":{"description":"OK"}}}}}}).to_string();
            let mut evidence = imported_contract_text(&text);
            assert!(evidence.contracts.contains_key("api/api.yaml"));
            assert!(evidence.sources.values().any(|s| s.file == "api/api.yaml"));
            enrich(&mut evidence).unwrap();
            let scope = evidence
                .observations
                .values()
                .find(|o| o.kind == "CONTRACT_SCOPE")
                .unwrap();
            assert_eq!(
                scope.normalized["inputs"]["api/api.yaml"]["status"],
                "DECLARED_OPENAPI"
            );
            let operation = evidence
                .observations
                .values()
                .find(|o| o.kind == "CONTRACT_OPERATION")
                .unwrap();
            assert_eq!(
                operation.normalized["readerSelection"]["selectionMode"],
                "MALFORMED_VERSION_ASSUMED"
            );
            assert!(operation.normalized["openapi"].is_null());
            assert!(
                evidence
                    .boundaries
                    .iter()
                    .any(|b| b == "OPENAPI_MALFORMED_VERSION_ASSUMED")
            );
            assert!(
                !scope
                    .normalized
                    .to_string()
                    .contains("private-version-value")
            );
            assert!(
                !operation
                    .normalized
                    .to_string()
                    .contains("private-version-value")
            );
        }
        let mut evidence = imported_contract_text(
            "paths:\n  /orders:\n    get:\n      responses: {'200': {description: OK}}\n",
        );
        enrich(&mut evidence).unwrap();
        let operation = evidence
            .observations
            .values()
            .find(|o| o.kind == "CONTRACT_OPERATION")
            .unwrap();
        assert_eq!(
            operation.normalized["readerSelection"]["selectionMode"],
            "MISSING_VERSION_ASSUMED"
        );
        assert!(
            evidence
                .boundaries
                .iter()
                .any(|b| b == "OPENAPI_MISSING_VERSION_ASSUMED")
        );
        let scope = evidence
            .observations
            .values()
            .find(|o| o.kind == "CONTRACT_SCOPE")
            .unwrap();
        assert_eq!(
            scope.normalized["inputs"]["api/types.yaml"]["status"],
            "REGISTERED_REFERENCE_DOCUMENT"
        );
        assert!(scope.normalized["inputs"]["api/types.yaml"]["readerSelection"].is_null());
    }

    #[test]
    fn parse_failures_and_unreadable_shapes_are_rejected_after_attempt() {
        for (text, code) in [
            ("openapi: [broken\n", "INVALID_CONTRACT_JSON_YAML"),
            ("{\"paths\":", "INVALID_CONTRACT_JSON_YAML"),
            ("openapi: 3.2.0\npaths: []\n", "UNREADABLE_CONTRACT_PATHS"),
            (
                "openapi: 4.0.0\npaths: private-value\n",
                "UNREADABLE_CONTRACT_PATHS",
            ),
            ("paths: {'/orders': null}", "UNREADABLE_CONTRACT_PATHS"),
            (
                "paths: {'/orders': {'$ref': []}}",
                "UNREADABLE_CONTRACT_PATHS",
            ),
            (
                "paths: {'/orders': {get: null}}",
                "UNREADABLE_CONTRACT_PATHS",
            ),
            ("paths: {'/orders': {get: []}}", "UNREADABLE_CONTRACT_PATHS"),
            (
                "paths: {'/orders': {post: 17}}",
                "UNREADABLE_CONTRACT_PATHS",
            ),
        ] {
            let mut evidence = imported_contract_text(text);
            enrich(&mut evidence).unwrap();
            assert!(!evidence.contracts.contains_key("api/api.yaml"), "{text}");
            assert!(evidence.sources.values().any(|s| s.file == "api/api.yaml"));
            assert!(
                evidence.boundaries.iter().any(|b| b.starts_with(code)),
                "{code}: {:?}",
                evidence.boundaries
            );
            assert!(
                evidence
                    .observations
                    .values()
                    .all(|o| o.kind != "CONTRACT_OPERATION")
            );
            if code != "INVALID_CONTRACT_JSON_YAML" {
                // Cached or supplied evidence must use the same structural checks as import.
                evidence.contracts.insert(
                    "api/api.yaml".into(),
                    serde_yaml_ng::from_str(text).unwrap(),
                );
                enrich(&mut evidence).unwrap();
                assert!(
                    evidence
                        .observations
                        .values()
                        .all(|o| o.kind != "CONTRACT_OPERATION")
                );
            }
        }
    }

    #[test]
    fn registered_boolean_and_array_reference_roots_resolve_without_api_admission() {
        for (reference, fragment, expected) in [
            ("true", "", json!(true)),
            ("false", "", json!(false)),
            ("[{\"type\":\"string\"}]", "#/0", json!({"type":"string"})),
        ] {
            let text = json!({"openapi":"3.2.0","paths":{"/orders":{"get":{"responses":{"200":{"description":"OK","content":{"application/json":{"schema":{"$ref":format!("types.yaml{fragment}")}}}}}}}}}).to_string();
            let mut evidence = imported_contract_with_reference(&text, reference);
            enrich(&mut evidence).unwrap();
            assert_eq!(
                evidence.contracts["api/types.yaml"],
                serde_json::from_str::<Value>(reference).unwrap()
            );
            let scope = evidence
                .observations
                .values()
                .find(|o| o.kind == "CONTRACT_SCOPE")
                .unwrap();
            assert_eq!(
                scope.normalized["inputs"]["api/types.yaml"]["status"],
                "REGISTERED_REFERENCE_DOCUMENT"
            );
            assert!(scope.normalized["inputs"]["api/types.yaml"]["readerSelection"].is_null());
            let operations: Vec<_> = evidence
                .observations
                .values()
                .filter(|o| o.kind == "CONTRACT_OPERATION")
                .collect();
            assert_eq!(operations.len(), 1);
            let operation = operations[0];
            assert_eq!(
                operation.normalized["operation"]["responses"]["200"]["content"]["application/json"]
                    ["schema"],
                expected
            );
            assert_eq!(operation.source_ids.len(), 2);
            assert!(
                operation
                    .source_ids
                    .iter()
                    .any(|id| evidence.sources[id].file == "api/types.yaml")
            );
            assert!(
                !operation.normalized["boundaries"]
                    .to_string()
                    .contains("UNREGISTERED_OR_UNAVAILABLE_CONTRACT_REFERENCE")
            );
            if expected.is_boolean() {
                assert!(
                    operation.normalized["boundaries"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .any(|b| b == "OPENAPI_BOOLEAN_SCHEMA_NOT_INTERPRETED")
                );
            }
        }
        for reference in ["null", "[1,2,3]", "\"fragment\"", "17"] {
            let mut evidence = imported_contract_with_reference("paths: {}", reference);
            enrich(&mut evidence).unwrap();
            assert!(evidence.contracts.contains_key("api/types.yaml"));
            assert!(
                evidence
                    .observations
                    .values()
                    .all(|o| o.kind != "CONTRACT_OPERATION")
            );
        }
    }

    #[test]
    fn reference_fragment_roots_cannot_be_misinterpreted_as_api_path_items() {
        let text = json!({"openapi":"3.2.0","paths":{"/orders":{"$ref":"types.yaml"}}}).to_string();
        for reference in ["true", "false", "[]", "null", "17"] {
            let mut evidence = imported_contract_with_reference(&text, reference);
            enrich(&mut evidence).unwrap();
            assert!(evidence.contracts.contains_key("api/types.yaml"));
            assert!(
                evidence
                    .observations
                    .values()
                    .all(|o| o.kind != "CONTRACT_OPERATION")
            );
            assert!(
                evidence
                    .boundaries
                    .iter()
                    .any(|b| b == "UNREADABLE_RESOLVED_CONTRACT_PATH_ITEM")
            );
        }
    }

    #[test]
    fn future_path_item_fields_are_retained_with_a_boundary_while_known_operations_are_read() {
        let text = json!({"openapi":"3.2.0","paths":{"/orders":{"get":{"responses":{"200":{"description":"OK"}}},"private-future-method":{"responses":{"200":{"description":"Future"}}}}}}).to_string();
        let mut evidence = imported_contract_text(&text);
        enrich(&mut evidence).unwrap();
        let operations: Vec<_> = evidence
            .observations
            .values()
            .filter(|o| o.kind == "CONTRACT_OPERATION")
            .collect();
        assert_eq!(operations.len(), 1);
        assert_eq!(operations[0].normalized["method"], "GET");
        let code = "OPENAPI_UNKNOWN_PATH_ITEM_FIELDS_RETAINED_NOT_INTERPRETED";
        assert!(evidence.boundaries.iter().any(|b| b == code));
        assert!(
            operations[0].normalized["boundaries"]
                .as_array()
                .unwrap()
                .iter()
                .any(|b| b == code)
        );
        assert!(
            evidence.contracts["api/api.yaml"]["paths"]["/orders"]
                .get("private-future-method")
                .is_some()
        );
        assert!(
            !operations[0].normalized["boundaries"]
                .to_string()
                .contains("private-future-method")
        );
    }

    #[test]
    fn unreadable_resolved_path_items_do_not_create_operations() {
        let text = json!({"openapi":"3.99.7","paths":{"/orders":{"$ref":"#/components/pathItems/Broken"}},"components":{"pathItems":{"Broken":{"get":[]}}}}).to_string();
        let mut evidence = imported_contract_text(&text);
        enrich(&mut evidence).unwrap();
        assert!(
            evidence
                .observations
                .values()
                .all(|o| o.kind != "CONTRACT_OPERATION")
        );
        assert!(
            evidence
                .boundaries
                .iter()
                .any(|b| b == "UNREADABLE_RESOLVED_CONTRACT_PATH_ITEM")
        );
        let scope = evidence
            .observations
            .values()
            .find(|o| o.kind == "CONTRACT_SCOPE")
            .unwrap();
        assert!(
            scope.normalized["boundaries"]
                .as_array()
                .unwrap()
                .iter()
                .any(|b| b == "UNREADABLE_RESOLVED_CONTRACT_PATH_ITEM")
        );
    }

    #[test]
    fn openapi_31_retains_path_operations_with_explicit_feature_boundaries() {
        let schema = json!({"$schema":"https://private.invalid/private-dialect", "type":["string","null"], "unevaluatedProperties":false, "patternProperties":{"private-name":{"type":"null"}}, "properties":{"flag":true}});
        let text = json!({"openapi":"3.1.2","jsonSchemaDialect":"https://private.invalid/private-dialect", "webhooks":{"private-hook":{"post":{"responses":{"200":{"description":"OK"}}}}},"paths":{"/orders":{"get":{"responses":{"200":{"description":"OK","content":{"application/json":{"schema":{"$ref":"#/components/schemas/Result"}}}}}}}},"components":{"schemas":{"Result":schema}}}).to_string();
        let mut evidence = imported_contract_text(&text);
        enrich(&mut evidence).unwrap();
        let operations: Vec<_> = evidence
            .observations
            .values()
            .filter(|o| o.kind == "CONTRACT_OPERATION")
            .collect();
        assert_eq!(operations.len(), 1);
        assert_eq!(
            operations[0].normalized["operation"]["responses"]["200"]["content"]["application/json"]
                ["schema"],
            schema
        );
        let scope = evidence
            .observations
            .values()
            .find(|o| o.kind == "CONTRACT_SCOPE")
            .unwrap();
        for code in [
            "OPENAPI_WEBHOOKS_NOT_SOURCE_MAPPED",
            "OPENAPI_SCHEMA_DIALECT_NOT_INTERPRETED",
            "OPENAPI_SCHEMA_TYPE_UNIONS_NOT_INTERPRETED",
            "OPENAPI_SCHEMA_NULL_TYPE_NOT_INTERPRETED",
            "OPENAPI_JSON_SCHEMA_KEYWORDS_NOT_INTERPRETED",
            "OPENAPI_BOOLEAN_SCHEMA_NOT_INTERPRETED",
        ] {
            assert!(
                evidence.boundaries.iter().any(|b| b == code),
                "{code}: {:?}",
                evidence.boundaries
            );
            assert!(
                scope.normalized["inputs"]["api/api.yaml"]["boundaries"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|b| b == code)
            );
        }
        let gaps = operations[0].normalized["boundaries"].to_string();
        for code in [
            "OPENAPI_SCHEMA_DIALECT_NOT_INTERPRETED",
            "OPENAPI_SCHEMA_TYPE_UNIONS_NOT_INTERPRETED",
            "OPENAPI_SCHEMA_NULL_TYPE_NOT_INTERPRETED",
            "OPENAPI_JSON_SCHEMA_KEYWORDS_NOT_INTERPRETED",
            "OPENAPI_BOOLEAN_SCHEMA_NOT_INTERPRETED",
        ] {
            assert!(gaps.contains(code), "{code}: {gaps}");
        }
        for gaps in [serde_json::to_string(&evidence.boundaries).unwrap(), gaps] {
            assert!(!gaps.contains("private"), "{gaps}");
            assert!(!gaps.contains("api/api.yaml"), "{gaps}");
        }
    }

    #[test]
    fn schema_feature_inspection_ignores_instance_data_and_property_names() {
        let document = json!({"openapi":"3.1.2","components":{"schemas":{"Result":{"type":"object","additionalProperties":false,"properties":{"const":{"type":"string","additionalProperties":true},"type":{"type":"string"}},"example":{"type":null,"$schema":"private","unevaluatedProperties":false},"default":{"type":["null"]},"enum":[{"$schema":"private"}]}}}});
        assert!(feature_boundaries(&document).is_empty());
    }

    #[test]
    fn expanding_reference_graph_has_a_shared_value_budget() {
        let documents = BTreeMap::from([(
            "api.yaml".into(),
            json!({"Leaf":{"type":"object","properties":{"id":{"type":"integer"}}}}),
        )]);
        let mut resolver = Resolver {
            documents: &documents,
            seen: BTreeSet::new(),
            gaps: BTreeSet::new(),
            files: BTreeSet::new(),
            remaining: 8,
        };
        assert!(
            resolver
                .resolve(&json!([{"$ref":"#/Leaf"},{"$ref":"#/Leaf"}]), "api.yaml", 0)
                .is_err()
        );
    }
}
