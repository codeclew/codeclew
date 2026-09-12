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
pub const TESTED_VERSIONS: &[&str] = &["3.0.0", "3.0.3"];
const MAX_FILE: usize = 2 * 1024 * 1024;

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
        let version = value.get("openapi");
        let supported = version.is_none()
            || version
                .and_then(Value::as_str)
                .is_some_and(|v| TESTED_VERSIONS.contains(&v));
        let status = if !supported {
            "UNSUPPORTED_CONTRACT_VERSION"
        } else if version.is_none() {
            "REGISTERED_REFERENCE_DOCUMENT"
        } else {
            "DECLARED_OPENAPI"
        };
        if !supported {
            evidence
                .boundaries
                .push(format!("UNSUPPORTED_CONTRACT_VERSION:{path}"));
        }
        inventory.insert(
            path.clone(),
            json!({"digest":hash,"version":version,"status":status}),
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
        // Unsupported roots remain retained declarations, never interpreted as supported operations.
        if supported {
            evidence.contracts.insert(path.clone(), value);
        }
    }
    let normalized = json!({"schema":SCHEMA,"authority":"DECLARED_OPENAPI","inputs":inventory,"implementationDigest":canonical::hash_bytes(include_bytes!("contracts.rs")),"limitations":["DECLARATIONS_NOT_RUNTIME_ENFORCEMENT","NO_IMPLICIT_NETWORK","NO_SCHEMA_INSTANCE_VALIDATION","CALLBACKS_RETAINED_NOT_SOURCE_MAPPED"]});
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
    for (file, document) in &evidence.contracts {
        if !document["openapi"]
            .as_str()
            .is_some_and(|v| TESTED_VERSIONS.contains(&v))
        {
            continue;
        }
        for (path, item) in document["paths"].as_object().into_iter().flatten() {
            let mut resolver = Resolver {
                documents: &evidence.contracts,
                seen: BTreeSet::new(),
                gaps: BTreeSet::new(),
                files: BTreeSet::from([file.clone()]),
                remaining: budget,
            };
            let item = resolver.resolve(item, file, 0)?;
            for method in [
                "get", "put", "post", "delete", "options", "head", "patch", "trace",
            ] {
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
                let normalized = json!({"schema":SCHEMA,"entrypoint":entry.map(|e|&e.id),"sourceMapping":mapping,"method":method.to_uppercase(),"path":path,"operation":operation,"parameters":parameters.values().collect::<Vec<_>>(),"security":security,"securitySchemes":schemes,"servers":servers,"declaredSource":file,"openapi":document["openapi"],"boundaries":resolver.gaps,"authority":"DECLARED_OPENAPI","runtimeEnforcement":"UNVERIFIED"});
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
