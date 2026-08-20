use crate::canonical;
use crate::error::{ClewError, ErrorCode};
use crate::model::ThreadIr;
use serde_json::{Map, Value, json};
use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::Command;
use walkdir::WalkDir;

// A bounded cross-file adapter hook commonly needs one workflow plus its
// request, result, runtime, and return-contract declarations. Keep enough
// authority-bearing surfaces for that closure while retaining a small hard
// cap independent of the stdout byte budget.
const MAX_EDIT_SURFACES: usize = 8;
const MAX_CONTRACTS: usize = 2;
const MAX_TESTS: usize = 1;
const MAX_EXECUTION_EDGES: usize = 4;

#[derive(Clone, Debug)]
struct SourceFile {
    path: String,
    source: String,
    is_test: bool,
}

#[derive(Clone, Debug)]
struct Candidate {
    declaration: Value,
    source_text: String,
    line_start: usize,
    line_end: usize,
    score: usize,
    reasons: BTreeSet<String>,
}

#[derive(Clone, Debug)]
struct TaskRequirement {
    term: String,
    kind: String,
    candidate_ids: Vec<String>,
    evidence_paths: Vec<String>,
    satisfied: bool,
}

#[derive(Clone, Debug)]
pub struct TaskContextSelection {
    files: Vec<SourceFile>,
    catalog: Vec<Candidate>,
    intent_tokens: BTreeSet<String>,
    goal_tokens: BTreeSet<String>,
    explicit_owners: BTreeSet<String>,
    requirements: Vec<TaskRequirement>,
    required_candidate_ids: BTreeSet<String>,
    entrypoint_candidate_ids: BTreeSet<String>,
    explicit_candidate_ids: BTreeSet<String>,
    requires_tests: bool,
}

impl TaskContextSelection {
    pub fn root_symbols(&self, limit: usize) -> Vec<String> {
        let mut roots = Vec::<String>::new();
        // A named entrypoint is the execution boundary of the task. Exact
        // declarations remain mandatory edit surfaces, but they must not
        // displace the entrypoint from the bounded semantic graph.
        roots.extend(self.catalog.iter().filter_map(|candidate| {
            let id = candidate.declaration["declarationId"].as_str()?;
            self.entrypoint_candidate_ids
                .contains(id)
                .then(|| {
                    candidate.declaration["legacySymbolId"]
                        .as_str()
                        .map(str::to_owned)
                })
                .flatten()
        }));
        // An explicitly named owner is the strongest task boundary. Prefer its
        // goal-bearing member over helper functions that happen to be terms;
        // the call graph closes over those helpers without spending another
        // expensive graph root.
        for owner in &self.explicit_owners {
            if let Some(symbol) = self
                .catalog
                .iter()
                .filter(|candidate| {
                    candidate.score > 0
                        && candidate.declaration["kind"]
                            .as_str()
                            .is_some_and(|kind| kind.contains("Function"))
                        && candidate
                            .declaration
                            .pointer("/symbolIdentity/containingDeclarations")
                            .and_then(Value::as_array)
                            .into_iter()
                            .flatten()
                            .filter_map(Value::as_str)
                            .any(|candidate_owner| candidate_owner == owner)
                })
                .filter_map(|candidate| candidate.declaration["legacySymbolId"].as_str())
                .max_by_key(|symbol| {
                    self.catalog
                        .iter()
                        .find(|candidate| {
                            candidate.declaration["legacySymbolId"].as_str() == Some(*symbol)
                        })
                        .map(|candidate| {
                            let name = candidate.declaration["name"]
                                .as_str()
                                .unwrap_or_default()
                                .to_lowercase();
                            (
                                goal_name_score(&name, &self.goal_tokens),
                                self.goal_tokens
                                    .iter()
                                    .filter(|token| {
                                        candidate
                                            .source_text
                                            .to_lowercase()
                                            .contains(token.as_str())
                                    })
                                    .count(),
                                candidate
                                    .reasons
                                    .iter()
                                    .filter(|reason| reason.starts_with("body:"))
                                    .count(),
                                candidate.score,
                            )
                        })
                        .unwrap_or_default()
                })
            {
                roots.push(symbol.to_owned());
            }
        }
        let mut ranked = self
            .catalog
            .iter()
            .filter(|candidate| {
                candidate.score > 0
                    && candidate.declaration["kind"]
                        .as_str()
                        .is_some_and(|kind| kind.contains("Function"))
            })
            .collect::<Vec<_>>();
        ranked.sort_by_key(|candidate| Reverse(root_candidate_rank(candidate, self)));
        roots.extend(ranked.into_iter().filter_map(|candidate| {
            candidate.declaration["legacySymbolId"]
                .as_str()
                .map(str::to_owned)
        }));
        let mut unique = Vec::<String>::new();
        for root in roots {
            if !unique.iter().any(|existing| existing == &root) {
                unique.push(root);
            }
            if unique.len() >= limit {
                break;
            }
        }
        unique
    }

    pub fn followup_symbols(&self, resolutions: &[Value], limit: usize) -> Vec<String> {
        let resolved = resolutions
            .iter()
            .filter_map(|resolution| {
                resolution
                    .pointer("/declaration/legacySymbolId")
                    .and_then(Value::as_str)
                    .map(normalize_symbol)
            })
            .collect::<BTreeSet<_>>();
        let mut ranked = BTreeMap::<String, usize>::new();
        for call in resolutions
            .iter()
            .flat_map(|resolution| resolution["resolvedCalls"].as_array().into_iter().flatten())
        {
            let Some(symbol) = call["symbol"].as_str() else {
                continue;
            };
            let normalized = normalize_symbol(symbol);
            if resolved.contains(&normalized) {
                continue;
            }
            let Some(candidate) = self.catalog.iter().find(|candidate| {
                let legacy = normalize_symbol(
                    candidate.declaration["legacySymbolId"]
                        .as_str()
                        .unwrap_or_default(),
                );
                symbol_matches_declaration(&normalized, &legacy, &candidate.declaration)
            }) else {
                continue;
            };
            if !candidate.declaration["kind"]
                .as_str()
                .is_some_and(|kind| kind.contains("Function"))
            {
                continue;
            }
            let parameter_score = call["argumentToParameter"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|mapping| {
                    let parameter = mapping["parameter"]
                        .as_str()
                        .unwrap_or_default()
                        .to_lowercase();
                    if self.intent_tokens.contains(&parameter) {
                        900
                    } else {
                        self.intent_tokens
                            .iter()
                            .filter(|token| token.len() >= 4 && parameter.contains(token.as_str()))
                            .count()
                            * 80
                    }
                })
                .sum::<usize>();
            let score = call_declaration_rank(candidate, &self.intent_tokens)
                + parameter_score
                + candidate.score;
            let symbol = candidate.declaration["legacySymbolId"]
                .as_str()
                .unwrap_or_default()
                .to_owned();
            ranked
                .entry(symbol)
                .and_modify(|existing| *existing = (*existing).max(score))
                .or_insert(score);
        }
        let mut ranked = ranked.into_iter().collect::<Vec<_>>();
        ranked.sort_by_key(|(symbol, score)| (Reverse(*score), symbol.clone()));
        let mut symbols = ranked
            .into_iter()
            .filter(|(_, score)| *score > 0)
            .map(|(symbol, _)| symbol)
            .take(limit)
            .collect::<Vec<_>>();
        if symbols.len() < limit {
            for symbol in self.root_symbols(self.catalog.len()) {
                let normalized = normalize_symbol(&symbol);
                if resolved.contains(&normalized) || symbols.iter().any(|item| item == &symbol) {
                    continue;
                }
                symbols.push(symbol);
                if symbols.len() >= limit {
                    break;
                }
            }
        }
        symbols
    }
}

fn root_candidate_rank(candidate: &Candidate, selection: &TaskContextSelection) -> usize {
    let name = candidate.declaration["name"]
        .as_str()
        .unwrap_or_default()
        .to_lowercase();
    let source = candidate.source_text.to_lowercase();
    let goal_source_hits = selection
        .goal_tokens
        .iter()
        .filter(|token| token.len() >= 2 && source.contains(token.as_str()))
        .count();
    let intent_source_hits = selection
        .intent_tokens
        .iter()
        .filter(|token| token.len() >= 4 && source.contains(token.as_str()))
        .count();
    goal_name_score(&name, &selection.goal_tokens) * 1_000
        + goal_source_hits * 120
        + intent_source_hits * 10
        + candidate.score
}

fn goal_name_score(name: &str, tokens: &BTreeSet<String>) -> usize {
    tokens
        .iter()
        .map(|token| {
            if token == name || token.strip_suffix('d') == Some(name) {
                3
            } else if token.len() >= 6 && name.contains(token.as_str()) {
                1
            } else {
                0
            }
        })
        .max()
        .unwrap_or_default()
}

pub fn select(
    repo: &Path,
    index_facts: &Value,
    terms: &[String],
    intent: &str,
) -> Result<TaskContextSelection, ClewError> {
    let files = scan_kotlin_sources(repo)?;
    let sources = files
        .iter()
        .map(|file| (file.path.as_str(), file.source.as_str()))
        .collect::<BTreeMap<_, _>>();
    let intent_tokens = task_tokens(intent, terms);
    let goal_tokens = task_tokens(primary_intent(intent), terms);
    let normalized_terms = terms
        .iter()
        .filter(|term| !looks_like_constant(term))
        .flat_map(|term| split_identifier_tokens(term))
        .flat_map(token_variants)
        .collect::<Vec<_>>();
    let explicit_owners = index_facts["files"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|file| file["declarations"].as_array().into_iter().flatten())
        .filter(|declaration| {
            declaration["kind"]
                .as_str()
                .is_some_and(|kind| kind.contains("Class"))
        })
        .filter_map(|declaration| {
            let name = declaration["name"].as_str()?;
            normalized_terms
                .iter()
                .any(|term| term == &name.to_lowercase())
                .then(|| name.to_owned())
        })
        .collect::<BTreeSet<_>>();

    let mut catalog = Vec::new();
    for file in index_facts["files"].as_array().into_iter().flatten() {
        let path = file["path"].as_str().unwrap_or_default();
        let Some(source) = sources.get(path) else {
            continue;
        };
        for declaration in file["declarations"].as_array().into_iter().flatten() {
            let start = declaration["rangeStart"].as_u64().unwrap_or_default() as usize;
            let end = declaration["rangeEnd"].as_u64().unwrap_or_default() as usize;
            let (byte_start, byte_end) = utf16_range_to_bytes(source, start, end);
            let source_text = source.get(byte_start..byte_end).unwrap_or_default();
            let (score, reasons) = candidate_score(
                declaration,
                source_text,
                &normalized_terms,
                &intent_tokens,
                &explicit_owners,
            );
            let candidate = Candidate {
                declaration: declaration.clone(),
                source_text: source_text.to_owned(),
                line_start: line_number(source, byte_start),
                line_end: line_number(source, byte_end),
                score,
                reasons,
            };
            catalog.push(candidate.clone());
        }
    }
    let (requirements, required_candidate_ids, entrypoint_candidate_ids, explicit_candidate_ids) =
        derive_requirements(&files, &catalog, terms);
    let requires_tests = intent_requests_tests(intent)
        || requirements
            .iter()
            .any(|requirement| requirement.kind == "TEST_SURFACE");
    Ok(TaskContextSelection {
        files,
        catalog,
        intent_tokens,
        goal_tokens,
        explicit_owners,
        requirements,
        required_candidate_ids,
        entrypoint_candidate_ids,
        explicit_candidate_ids,
        requires_tests,
    })
}

fn derive_requirements(
    files: &[SourceFile],
    catalog: &[Candidate],
    terms: &[String],
) -> (
    Vec<TaskRequirement>,
    BTreeSet<String>,
    BTreeSet<String>,
    BTreeSet<String>,
) {
    let mut requirements = Vec::new();
    let mut required = BTreeSet::new();
    let mut entrypoints = BTreeSet::new();
    let mut explicit = BTreeSet::new();
    let test_paths = files
        .iter()
        .filter(|file| file.is_test)
        .map(|file| file.path.as_str())
        .collect::<BTreeSet<_>>();
    for term in terms {
        let exact = catalog
            .iter()
            .filter(|candidate| is_surface_declaration(candidate))
            .filter(|candidate| {
                candidate.declaration["file"]
                    .as_str()
                    .is_none_or(|path| !test_paths.contains(path))
            })
            .filter(|candidate| {
                candidate.declaration["name"]
                    .as_str()
                    .is_some_and(|name| name == term)
            })
            .collect::<Vec<_>>();
        if !exact.is_empty() {
            let ids = exact
                .iter()
                .filter_map(|candidate| candidate_id(candidate))
                .collect::<Vec<_>>();
            required.extend(ids.iter().cloned());
            explicit.extend(ids.iter().cloned());
            requirements.push(TaskRequirement {
                term: term.clone(),
                kind: "EXPLICIT_DECLARATION".into(),
                candidate_ids: ids,
                evidence_paths: Vec::new(),
                satisfied: true,
            });
            continue;
        }

        // Test files are evidence for the separately anchored TEST_SURFACE.
        // They are not production edit surfaces in the active compilation.
        let matching_files = files
            .iter()
            .filter(|file| !file.is_test)
            .filter(|file| {
                Path::new(&file.path)
                    .file_stem()
                    .and_then(|stem| stem.to_str())
                    .is_some_and(|stem| stem.eq_ignore_ascii_case(term))
            })
            .collect::<Vec<_>>();
        let matching_paths = matching_files
            .iter()
            .map(|file| file.path.as_str())
            .collect::<BTreeSet<_>>();
        if !matching_paths.is_empty() {
            let in_file = catalog
                .iter()
                .filter(|candidate| is_surface_declaration(candidate))
                .filter(|candidate| {
                    candidate.declaration["file"]
                        .as_str()
                        .is_some_and(|path| matching_paths.contains(path))
                })
                .collect::<Vec<_>>();
            let main = in_file.iter().copied().find(|candidate| {
                candidate.declaration["name"]
                    .as_str()
                    .is_some_and(|name| name == "main")
            });
            let selected = main.or_else(|| {
                in_file
                    .iter()
                    .copied()
                    .max_by_key(|candidate| (candidate.score, candidate.source_text.len()))
            });
            let ids = selected
                .into_iter()
                .filter_map(candidate_id)
                .collect::<Vec<_>>();
            required.extend(ids.iter().cloned());
            if main.is_some() {
                entrypoints.extend(ids.iter().cloned());
            } else {
                explicit.extend(ids.iter().cloned());
            }
            requirements.push(TaskRequirement {
                term: term.clone(),
                kind: if main.is_some() {
                    "ENTRYPOINT_FILE"
                } else {
                    "FILE_SURFACE"
                }
                .into(),
                satisfied: !ids.is_empty(),
                candidate_ids: ids,
                evidence_paths: Vec::new(),
            });
            continue;
        }

        let matching_test_paths = files
            .iter()
            .filter(|file| exact_test_file_stem(file, term) || exact_test_declaration(file, term))
            .map(|file| file.path.clone())
            .collect::<Vec<_>>();
        if !matching_test_paths.is_empty() {
            requirements.push(TaskRequirement {
                term: term.clone(),
                kind: "TEST_SURFACE".into(),
                candidate_ids: Vec::new(),
                evidence_paths: matching_test_paths,
                satisfied: true,
            });
            continue;
        }

        // Unanchored text is useful for discovery, but cannot prove that an
        // exact requested surface is safe to edit.  Keep the requirement
        // visible and fail closed until it has an immutable declaration/file
        // target.
        requirements.push(TaskRequirement {
            term: term.clone(),
            kind: "EVIDENCE_TERM".into(),
            candidate_ids: Vec::new(),
            evidence_paths: Vec::new(),
            satisfied: false,
        });
    }
    (requirements, required, entrypoints, explicit)
}

fn candidate_id(candidate: &Candidate) -> Option<String> {
    candidate.declaration["declarationId"]
        .as_str()
        .map(str::to_owned)
}

fn is_surface_declaration(candidate: &Candidate) -> bool {
    candidate.declaration["kind"].as_str().is_some_and(|kind| {
        kind.contains("Function") || kind.contains("Class") || kind.contains("ObjectDeclaration")
    })
}

fn exact_test_file_stem(file: &SourceFile, term: &str) -> bool {
    file.is_test
        && Path::new(&file.path)
            .file_stem()
            .and_then(|stem| stem.to_str())
            .is_some_and(|stem| stem.eq_ignore_ascii_case(term))
}

fn exact_test_declaration(file: &SourceFile, term: &str) -> bool {
    file.is_test
        && file
            .source
            .lines()
            .any(|line| test_function_name(line) == Some(term))
}

fn requirement_is_satisfied(requirement: &TaskRequirement, tests: &[Value]) -> bool {
    requirement.satisfied
        && (requirement.kind != "TEST_SURFACE"
            || requirement.evidence_paths.iter().any(|path| {
                tests
                    .iter()
                    .any(|test| test["path"].as_str() == Some(path.as_str()))
            }))
}

fn intent_requests_tests(intent: &str) -> bool {
    let lower = intent.to_lowercase();
    ["test", "coverage", "@displayname", "тест", "покрыт"]
        .into_iter()
        .any(|needle| lower.contains(needle))
}

fn looks_like_constant(term: &str) -> bool {
    let mut letters = term.chars().filter(|character| character.is_alphabetic());
    let letter_count = letters.clone().count();
    letter_count >= 2 && letters.all(|character| character.is_uppercase())
}

fn primary_intent(intent: &str) -> &str {
    let lower = intent.to_lowercase();
    [" without ", " preserve ", " do not ", " add regression "]
        .into_iter()
        .filter_map(|boundary| lower.find(boundary))
        .min()
        .and_then(|index| intent.get(..index))
        .unwrap_or(intent)
}

fn candidate_score(
    declaration: &Value,
    source: &str,
    terms: &[String],
    intent_tokens: &BTreeSet<String>,
    explicit_owners: &BTreeSet<String>,
) -> (usize, BTreeSet<String>) {
    let name = declaration["name"].as_str().unwrap_or_default();
    let name_lower = name.to_lowercase();
    let legacy_lower = declaration["legacySymbolId"]
        .as_str()
        .unwrap_or_default()
        .to_lowercase();
    let source_lower = source.to_lowercase();
    let kind = declaration["kind"].as_str().unwrap_or_default();
    let owners = declaration
        .pointer("/symbolIdentity/containingDeclarations")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect::<BTreeSet<_>>();
    let mut score = 0usize;
    let mut reasons = BTreeSet::new();
    for term in terms {
        if name_lower == *term {
            score += 220;
            reasons.insert(format!("exact:{term}"));
        } else if legacy_lower
            .split_once('(')
            .map_or(legacy_lower.as_str(), |(prefix, _)| prefix)
            .ends_with(&format!(".{term}"))
        {
            score += 180;
            reasons.insert(format!("symbol:{term}"));
        } else if name_lower.starts_with(term) {
            score += 90;
            reasons.insert(format!("name-prefix:{term}"));
        }
        if source_lower.contains(term) {
            score += 24;
            reasons.insert(format!("body:{term}"));
        }
    }
    let owner_match = owners.iter().any(|owner| explicit_owners.contains(*owner));
    if owner_match {
        score += 35;
        reasons.insert("member-of-explicit-owner".into());
    }
    for token in intent_tokens {
        if token.len() < 2 {
            continue;
        }
        if name_lower.contains(token) {
            score += 48;
            reasons.insert(format!("intent-name:{token}"));
        } else if source_lower.contains(token) {
            score += 3;
            reasons.insert(format!("intent-body:{token}"));
        }
    }
    if kind.contains("Function") {
        score += 20;
    }
    let directly_matched = reasons
        .iter()
        .any(|reason| reason.starts_with("exact:") || reason.starts_with("symbol:"));
    let task_member = owner_match
        && reasons
            .iter()
            .any(|reason| reason.starts_with("intent-name:") || reason.starts_with("body:"));
    let connected_function = kind.contains("Function")
        && reasons
            .iter()
            .any(|reason| reason.starts_with("intent-name:"))
        && reasons
            .iter()
            .filter(|reason| reason.starts_with("intent-body:"))
            .count()
            >= 2;
    let task_contract = kind.contains("Class")
        && reasons
            .iter()
            .filter(|reason| reason.starts_with("intent-body:"))
            .count()
            >= 2;
    if directly_matched || task_member || connected_function || task_contract {
        (score, reasons)
    } else {
        (0, BTreeSet::new())
    }
}

pub struct TaskContextBuild<'a> {
    pub repo: &'a Path,
    pub terms: &'a [String],
    pub intent: &'a str,
    pub compilation: &'a str,
    pub project: &'a Value,
    pub index_facts: &'a Value,
    pub selection: &'a TaskContextSelection,
    pub resolutions: &'a [Value],
    pub threads: &'a [ThreadIr],
    pub base_revision: &'a str,
    pub index_snapshot: &'a str,
    pub evidence_path: &'a Path,
    pub model_input_surfaces: &'a [Value],
    pub max_bytes: usize,
}

/// Resolve explicitly requested build/model inputs into immutable task
/// surfaces.  A model input is authority-bearing: unlike a source search hit,
/// it is only exposed when Git, the live filesystem, and OpenProject's exact
/// semantic input manifest all name the same bytes.
pub fn resolve_model_input_surfaces(
    repo: &Path,
    project: &Value,
    requested: &[String],
) -> Result<Vec<Value>, ClewError> {
    let manifest = verified_semantic_input_manifest(project)?;
    let entries = manifest
        .get("modelInputs")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            ClewError::new(
                ErrorCode::WorkerProtocolMismatch,
                "OpenProject semantic input manifest has no modelInputs array",
            )
        })?;
    let manifest_hash = project["semanticInputManifestHash"]
        .as_str()
        .expect("verified manifest hash");
    let mut seen = BTreeSet::new();
    let mut surfaces = Vec::with_capacity(requested.len());
    for raw in requested {
        let path = canonical_model_input_path(raw)?;
        if !seen.insert(path.to_owned()) {
            return Err(ClewError::new(
                ErrorCode::InvalidInput,
                format!("model input was requested more than once: {path}"),
            ));
        }
        require_tracked_regular_file(repo, path)?;
        let bytes = std::fs::read(repo.join(path)).map_err(|error| {
            ClewError::new(
                ErrorCode::InvalidInput,
                format!("cannot read model input {path}: {error}"),
            )
        })?;
        let source = std::str::from_utf8(&bytes).map_err(|_| {
            ClewError::new(
                ErrorCode::InvalidInput,
                format!("model input is not UTF-8: {path}"),
            )
        })?;
        let exact_hash = canonical::hash_bytes(&bytes);
        require_manifest_model_input(entries, path, &exact_hash)?;
        let target = json!({
            "anchorId":format!("model-input:{}", canonical::hash(&json!({"path":path,"hash":exact_hash})).map_err(|error| ClewError::new(ErrorCode::Internal,error.to_string()))?),
            "fileId":path,
            "exactTextHash":exact_hash,
            "syntaxKind":"MODEL_INPUT_FILE",
            "semanticInputManifestHash":manifest_hash,
        });
        surfaces.push(json!({
            "targetId":format!("M{}", surfaces.len() + 1),
            "path":path,
            "exactHash":exact_hash,
            "sourceText":source,
            "status":"REQUIRED",
            "required":true,
            "surfaceRequired":true,
            "modelInputTarget":target,
        }));
    }
    Ok(surfaces)
}

fn verified_semantic_input_manifest(project: &Value) -> Result<&Value, ClewError> {
    let manifest = project.get("semanticInputManifest").ok_or_else(|| {
        ClewError::new(
            ErrorCode::WorkerProtocolMismatch,
            "OpenProject has no semantic input manifest",
        )
    })?;
    let expected = project
        .get("semanticInputManifestHash")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            ClewError::new(
                ErrorCode::WorkerProtocolMismatch,
                "OpenProject has no semantic input manifest hash",
            )
        })?;
    let actual = canonical::hash(manifest)
        .map_err(|error| ClewError::new(ErrorCode::Internal, error.to_string()))?;
    if actual != expected {
        return Err(ClewError::new(
            ErrorCode::ProjectModelChanged,
            "OpenProject semantic input manifest hash is invalid",
        ));
    }
    Ok(manifest)
}

fn canonical_model_input_path(raw: &str) -> Result<&str, ClewError> {
    let path = Path::new(raw);
    let canonical = !raw.is_empty()
        && !raw.contains('\\')
        && !path.is_absolute()
        && path
            .components()
            .all(|component| matches!(component, std::path::Component::Normal(_)))
        && path
            .components()
            .map(|component| component.as_os_str())
            .collect::<PathBuf>()
            .as_os_str()
            == OsStr::new(raw);
    if !canonical {
        return Err(ClewError::new(
            ErrorCode::InvalidInput,
            format!("model input path is not canonical repository-relative UTF-8: {raw}"),
        ));
    }
    Ok(raw)
}

fn require_tracked_regular_file(repo: &Path, relative: &str) -> Result<(), ClewError> {
    let output = Command::new("git")
        .args(["ls-files", "--stage", "-z", "--", relative])
        .current_dir(repo)
        .output()
        .map_err(|error| ClewError::new(ErrorCode::InvalidInput, error.to_string()))?;
    if !output.status.success() {
        return Err(ClewError::new(
            ErrorCode::InvalidInput,
            format!("cannot verify tracked model input: {relative}"),
        ));
    }
    let records = output
        .stdout
        .split(|byte| *byte == 0)
        .filter(|record| !record.is_empty())
        .collect::<Vec<_>>();
    let exact_regular = records.len() == 1
        && records[0]
            .iter()
            .position(|byte| *byte == b'\t')
            .is_some_and(|separator| {
                let (stage, path_with_tab) = records[0].split_at(separator);
                let path = &path_with_tab[1..];
                let mut fields = stage.split(|byte| *byte == b' ');
                matches!(fields.next(), Some(b"100644" | b"100755"))
                    && fields.next().is_some()
                    && fields.next() == Some(b"0")
                    && fields.next().is_none()
                    && path == relative.as_bytes()
            });
    if !exact_regular {
        return Err(ClewError::new(
            ErrorCode::InvalidInput,
            format!("model input is not one exact tracked regular file: {relative}"),
        ));
    }

    let mut current = repo.to_path_buf();
    let components = Path::new(relative).components().collect::<Vec<_>>();
    for (index, component) in components.iter().enumerate() {
        current.push(component.as_os_str());
        let metadata = std::fs::symlink_metadata(&current).map_err(|error| {
            ClewError::new(
                ErrorCode::InvalidInput,
                format!(
                    "model input path is unavailable at {}: {error}",
                    current.display()
                ),
            )
        })?;
        if metadata.file_type().is_symlink()
            || (index + 1 == components.len() && !metadata.is_file())
            || (index + 1 < components.len() && !metadata.is_dir())
        {
            return Err(ClewError::new(
                ErrorCode::InvalidInput,
                format!("model input path is not a nonsymlink regular file: {relative}"),
            ));
        }
    }
    Ok(())
}

fn require_manifest_model_input(
    entries: &[Value],
    path: &str,
    exact_hash: &str,
) -> Result<(), ClewError> {
    let matches = entries
        .iter()
        .filter(|entry| entry.get("path").and_then(Value::as_str) == Some(path))
        .collect::<Vec<_>>();
    if matches.len() != 1 || matches[0].get("hash").and_then(Value::as_str) != Some(exact_hash) {
        return Err(ClewError::new(
            ErrorCode::ProjectModelChanged,
            format!("model input {path} is absent or has a different digest in OpenProject"),
        ));
    }
    Ok(())
}

pub fn build(input: TaskContextBuild<'_>) -> Result<(Value, Value), ClewError> {
    let TaskContextBuild {
        repo,
        terms,
        intent,
        compilation,
        project,
        index_facts,
        selection,
        resolutions,
        threads,
        base_revision,
        index_snapshot,
        evidence_path,
        model_input_surfaces,
        max_bytes,
    } = input;
    if max_bytes < 4_096 {
        return Err(ClewError::new(
            ErrorCode::InvalidInput,
            "--max-bytes must be at least 4096 for a task closure",
        ));
    }
    let root_symbols = resolutions
        .iter()
        .filter_map(|resolution| {
            resolution
                .pointer("/declaration/legacySymbolId")
                .and_then(Value::as_str)
        })
        .collect::<BTreeSet<_>>();
    let root_candidates = selection
        .catalog
        .iter()
        .filter(|candidate| {
            candidate.declaration["legacySymbolId"]
                .as_str()
                .is_some_and(|symbol| root_symbols.contains(symbol))
        })
        .collect::<Vec<_>>();
    let call_declarations = collect_call_declarations(index_facts, selection, resolutions);
    let required_candidates = selection
        .catalog
        .iter()
        .filter(|candidate| {
            candidate_id(candidate).is_some_and(|id| selection.required_candidate_ids.contains(&id))
        })
        .collect::<Vec<_>>();
    let contract_declarations = collect_contracts(
        index_facts,
        selection,
        resolutions,
        &call_declarations,
        &required_candidates,
    );
    let projection_fields = collect_projection_fields(selection, &call_declarations);
    let required_surface_overflow = required_candidates.len().saturating_sub(MAX_EDIT_SURFACES);
    let mut edit_candidates = required_candidates.clone();
    for candidate in &root_candidates {
        if !edit_candidates.iter().any(|existing| {
            existing.declaration["declarationId"] == candidate.declaration["declarationId"]
        }) {
            edit_candidates.push(candidate);
        }
    }
    for candidate in &call_declarations {
        if !edit_candidates.iter().any(|existing| {
            existing.declaration["declarationId"] == candidate.declaration["declarationId"]
        }) {
            edit_candidates.push(candidate);
        }
    }
    edit_candidates.sort_by_key(|candidate| {
        let id = candidate.declaration["declarationId"]
            .as_str()
            .unwrap_or_default();
        Reverse((
            usize::from(selection.entrypoint_candidate_ids.contains(id)) * 4
                + usize::from(selection.explicit_candidate_ids.contains(id)) * 3
                + usize::from(
                    root_symbols.contains(
                        candidate.declaration["legacySymbolId"]
                            .as_str()
                            .unwrap_or_default(),
                    ),
                ) * 2,
            surface_rank(candidate, &root_symbols, &selection.intent_tokens),
        ))
    });
    edit_candidates.truncate(MAX_EDIT_SURFACES);

    let body_anchors = resolutions
        .iter()
        .filter_map(|resolution| {
            Some((
                resolution
                    .pointer("/declaration/legacySymbolId")?
                    .as_str()?
                    .to_owned(),
                resolution.get("bodyAnchor")?.clone(),
            ))
        })
        .collect::<BTreeMap<_, _>>();
    let edit_surfaces = edit_candidates
        .iter()
        .enumerate()
        .map(|(index, candidate)| {
            let mut surface = compact_surface(
                candidate,
                body_anchors.get(
                    candidate.declaration["legacySymbolId"]
                        .as_str()
                        .unwrap_or_default(),
                ),
            );
            let id = candidate.declaration["declarationId"]
                .as_str()
                .unwrap_or_default();
            let explicitly_required = selection.required_candidate_ids.contains(id);
            let role = if selection.entrypoint_candidate_ids.contains(id) {
                "WORKFLOW"
            } else if selection.explicit_candidate_ids.contains(id) {
                "EXPLICIT_TARGET"
            } else if root_candidates.first().is_some_and(|root| {
                root.declaration["declarationId"] == candidate.declaration["declarationId"]
            }) {
                "WORKFLOW"
            } else if root_candidates.iter().any(|root| {
                root.declaration["declarationId"] == candidate.declaration["declarationId"]
            }) {
                "INTERMEDIARY"
            } else if is_contract_source(&candidate.source_text) {
                "OUTPUT_CONTRACT"
            } else if candidate.source_text.contains("@Query") {
                "DATA_SOURCE"
            } else {
                "DEPENDENCY"
            };
            surface["role"] = json!(role);
            surface["required"] = json!(explicitly_required || role == "WORKFLOW");
            surface["surfaceRequired"] = json!(true);
            surface["surfaceOrder"] = json!(index + 1);
            surface
        })
        .collect::<Vec<_>>();
    let contracts = contract_declarations
        .iter()
        .filter(|contract| {
            !edit_candidates.iter().any(|surface| {
                surface.declaration["declarationId"] == contract.declaration["declarationId"]
            })
        })
        .take(if selection.explicit_candidate_ids.is_empty() {
            1
        } else {
            MAX_CONTRACTS
        })
        .map(|candidate| compact_contract(candidate))
        .collect::<Vec<_>>();
    let execution_path = collect_execution_path(resolutions, index_facts, selection);
    let test_needles = task_needles(terms, intent, &root_candidates, &edit_candidates, selection);
    let mut tests = collect_anchored_tests(selection, &test_needles, terms);
    if selection.requires_tests {
        for test in &mut tests {
            test["required"] = json!(true);
        }
    }
    let validation_plan = validation_plan(project, &tests);
    let matched_terms = terms
        .iter()
        .filter(|term| {
            selection
                .files
                .iter()
                .any(|file| file.source.contains(term.as_str()))
        })
        .cloned()
        .collect::<BTreeSet<_>>();
    let unmatched_terms = terms
        .iter()
        .filter(|term| !matched_terms.contains(*term))
        .cloned()
        .collect::<Vec<_>>();
    let internal_calls = execution_path.len();
    let missing_internal_calls = 0usize;
    let mut boundaries = Vec::new();
    if root_candidates.is_empty() {
        boundaries.push(json!({"kind":"NO_FUNCTION_ROOT"}));
    }
    if missing_internal_calls > 0 {
        boundaries.push(json!({"kind":"UNRESOLVED_INTERNAL_CALLS","count":missing_internal_calls}));
    }
    for requirement in selection
        .requirements
        .iter()
        .filter(|requirement| !requirement_is_satisfied(requirement, &tests))
    {
        boundaries.push(json!({"kind":"UNSATISFIED_REQUIREMENT","term":requirement.term,"requirementKind":requirement.kind}));
    }
    if required_surface_overflow > 0 {
        boundaries.push(json!({"kind":"REQUIRED_SURFACE_LIMIT","omitted":required_surface_overflow,"limit":MAX_EDIT_SURFACES}));
    }
    if selection.requires_tests && tests.is_empty() {
        boundaries.push(json!({"kind":"MISSING_TEST_SURFACE"}));
    }
    let requirements = selection
        .requirements
        .iter()
        .map(|requirement| {
            let mut matches = requirement
                .candidate_ids
                .iter()
                .filter_map(|id| {
                    selection.catalog.iter().find(|candidate| {
                        candidate.declaration["declarationId"].as_str() == Some(id)
                    })
                })
                .map(|candidate| {
                    json!({
                        "name":candidate.declaration["name"],
                        "file":candidate.declaration["file"]
                    })
                })
                .collect::<Vec<_>>();
            matches.extend(
                requirement
                    .evidence_paths
                    .iter()
                    .map(|path| json!({"file":path})),
            );
            json!({
                "term":requirement.term,
                "kind":requirement.kind,
                "status":if requirement_is_satisfied(requirement, &tests) {"SATISFIED"} else {"UNSATISFIED"},
                "matches":matches
            })
        })
        .chain(selection.requires_tests.then(|| {
            json!({
                "kind":"TEST_SURFACE",
                "status":if tests.is_empty() {"UNSATISFIED"} else {"SATISFIED"},
                "matches":tests.iter().filter_map(|test|test["path"].as_str()).collect::<Vec<_>>()
            })
        }))
        .collect::<Vec<_>>();
    let thread_id = threads
        .first()
        .map(|thread| thread.thread_id.as_str())
        .unwrap_or_default();
    let available_roles = edit_surfaces
        .iter()
        .filter_map(|surface| surface["role"].as_str())
        .collect::<BTreeSet<_>>();
    let transient_transform_available =
        ["WORKFLOW", "INTERMEDIARY", "OUTPUT_CONTRACT", "DATA_SOURCE"]
            .into_iter()
            .all(|role| available_roles.contains(role))
            && contracts.len() == 1
            && tests.len() == 1
            && !projection_fields.is_empty();
    let transient_transform = if transient_transform_available {
        json!({
            "available":true,
            "kind":"PROPAGATE_TYPED_FIELDS",
            "schema":"semantic-task-goal/0.4",
            "fields":projection_fields.iter().filter_map(|field|field["name"].as_str()).collect::<Vec<_>>(),
            "goalShape":{
                "schema":"semantic-task-goal/0.4",
                "baseRevision":base_revision,
                "transform":{
                    "kind":"PROPAGATE_TYPED_FIELDS",
                    "fields":projection_fields.iter().filter_map(|field|field["name"].as_str()).collect::<Vec<_>>(),
                    "names":{"newContract":"<new identifier>","newProjection":"<new identifier>","imports":["<fully qualified field type import>"]}
                }
            },
            "required":"transform.kind/fields; names.newContract/newProjection/imports",
            "constraints":["newContract and newProjection must be distinct new top-level identifiers", "do not restate or rename resolved graph bindings"],
            "instruction":"Copy goalShape and replace only its type-name/import placeholders. The worker derives data-source, collection, loop item, sink bindings, identity field, and anchored test binding from full resolved evidence. Never add graph bindings, existing contract names, source text, substitutions, target IDs, or occurrence counts."
        })
    } else {
        json!({"available":false})
    };
    let full = json!({
        "schema":"semantic-task-context/0.2",
        "task":{"intent":intent,"terms":terms,"intentTokens":selection.intent_tokens,"matchedTerms":matched_terms,"unmatchedTerms":unmatched_terms,"requirements":requirements},
        "snapshot":{"baseRevision":base_revision,"projectModelHash":project["projectModelHash"],"indexSnapshot":index_snapshot,"compilerVersion":project["compilerVersion"],"compilation":compilation},
        "threadId":thread_id,
        "editSurfaces":edit_surfaces,
        "modelInputSurfaces":model_input_surfaces,
        "executionPath":execution_path,
        "projectionFields":projection_fields,
        "contracts":contracts,
        "tests":tests,
        "validationPlan":validation_plan,
        "editPlan":{
            "schema":"semantic-task-edit-plan/0.1",
            "threadId":thread_id,
            "baseRevision":base_revision,
            "operationShape":{"kind":"REWRITE_DECLARATION","target":{"targetId":"<emitted targetId>"},"old":"...","new":"..."},
            "modelInputOperationShape":{"kind":"REPLACE_MODEL_INPUT","target":{"targetId":"M1"},"newLines":["<complete file, one exact line per item>"]},
            "transientTransform":transient_transform,
            "instruction":"Use transientTransform when available. Otherwise use kind and target.targetId. For REWRITE_DECLARATION use S/C/T aliases, never a B-suffixed body alias; B aliases are only for REPLACE_FUNCTION_BODY. Replace an emitted M alias only with REPLACE_MODEL_INPUT and a complete newLines array; paths are never accepted. Multiline: oldLines/newLines/kotlinLines arrays; occurrence selects one match, occurrences edits all; same-target rewrites merge. Plan every necessary role, including INTERMEDIARY type flow. A typed contract must not become Any/Any?: required fields stay static and existing payloads assignable. projectionFields nullability is authoritative. Add top-level types only with CREATE_FILE."
        },
        "evidence":evidence_display(repo,evidence_path),
        "completeness":{
            "status":if boundaries.is_empty(){"COMPLETE_TASK"}else{"PARTIAL_TASK"},
            "boundaries":boundaries,
            "coverage":{"roots":root_candidates.len(),"resolvedCalls":execution_path.len(),"internalCalls":internal_calls,"contracts":contracts.len(),"tests":tests.len()},
            "stdoutLimitBytes":max_bytes,
            "omitted":{"editSurfaces":0,"modelInputSurfaces":0,"executionPath":0,"contracts":0,"tests":0,"sourceBytes":0}
        }
    });
    let bounded = enforce_budget(compact_targets_for_stdout(full.clone()), max_bytes)?;
    let evidence = json!({
        "schema":"semantic-task-context-evidence/0.2",
        "stdoutCompleteness":bounded["completeness"],
        "context":full,
        "project":project,
        "index":index_facts,
        "resolutions":resolutions,
        "threads":threads,
    });
    Ok((bounded, evidence))
}

fn compact_surface(candidate: &Candidate, body_anchor: Option<&Value>) -> Value {
    let declaration = &candidate.declaration;
    let declaration_target = declaration_target(candidate);
    let mut value = Map::from_iter([
        ("name".into(), declaration["name"].clone()),
        ("kind".into(), declaration["kind"].clone()),
        ("file".into(), declaration["file"].clone()),
        (
            "lines".into(),
            json!([candidate.line_start, candidate.line_end]),
        ),
        ("score".into(), json!(candidate.score)),
        ("reasons".into(), json!(candidate.reasons)),
        ("sourceText".into(), json!(candidate.source_text)),
        ("declarationTarget".into(), declaration_target),
    ]);
    if let Some(anchor) = body_anchor {
        let mut compact = anchor.clone();
        if let Some(object) = compact.as_object_mut() {
            object.remove("sourceText");
        }
        value.insert("bodyTarget".into(), compact);
    }
    Value::Object(value)
}

fn compact_contract(candidate: &Candidate) -> Value {
    let source_text = declaration_header(&candidate.source_text);
    json!({
        "name":candidate.declaration["name"],
        "kind":candidate.declaration["kind"],
        "file":candidate.declaration["file"],
        "lines":[candidate.line_start,candidate.line_end],
        "sourceText":source_text,
        "sourceProjection":"DECLARATION_HEADER",
        "required":true,
        "declarationTarget":declaration_target(candidate)
    })
}

fn declaration_header(source: &str) -> String {
    let mut parentheses = 0isize;
    let mut seen_declaration = false;
    for (offset, character) in source.char_indices() {
        match character {
            '(' => parentheses += 1,
            ')' => parentheses -= 1,
            '{' if seen_declaration && parentheses <= 0 => {
                return compact_header_text(&source[..offset]);
            }
            _ => {}
        }
        if character.is_alphabetic() {
            seen_declaration = true;
        }
    }
    compact_header_text(source)
}

fn compact_header_text(source: &str) -> String {
    source
        .lines()
        .map(|line| line.split_once("//").map_or(line, |(code, _)| code).trim())
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

fn declaration_target(candidate: &Candidate) -> Value {
    json!({
        "anchorId":candidate.declaration["declarationId"],
        "declarationId":candidate.declaration["declarationId"],
        "fileId":candidate.declaration["file"],
        "ownerSymbolId":candidate.declaration["symbolId"],
        "syntaxKind":candidate.declaration["kind"],
        "exactTextHash":canonical::hash_bytes(candidate.source_text.as_bytes()),
        "rangeHint":[candidate.declaration["rangeStart"],candidate.declaration["rangeEnd"]]
    })
}

fn collect_call_declarations<'a>(
    index_facts: &Value,
    selection: &'a TaskContextSelection,
    resolutions: &[Value],
) -> Vec<&'a Candidate> {
    let call_symbols = resolutions
        .iter()
        .flat_map(|resolution| resolution["resolvedCalls"].as_array().into_iter().flatten())
        .filter_map(|call| call["symbol"].as_str())
        .map(normalize_symbol)
        .collect::<BTreeSet<_>>();
    let known_ids = index_facts["files"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|file| file["declarations"].as_array().into_iter().flatten())
        .filter_map(|declaration| {
            let legacy = normalize_symbol(declaration["legacySymbolId"].as_str()?);
            call_symbols
                .iter()
                .any(|symbol| symbol_matches_declaration(symbol, &legacy, declaration))
                .then(|| {
                    declaration["declarationId"]
                        .as_str()
                        .unwrap_or_default()
                        .to_owned()
                })
        })
        .collect::<BTreeSet<_>>();
    let mut candidates = selection
        .catalog
        .iter()
        .filter(|candidate| {
            candidate.declaration["declarationId"]
                .as_str()
                .is_some_and(|id| known_ids.contains(id))
        })
        .collect::<Vec<_>>();
    candidates.sort_by_key(|candidate| {
        Reverse((
            call_declaration_rank(candidate, &selection.intent_tokens),
            candidate.declaration["name"]
                .as_str()
                .unwrap_or_default()
                .to_owned(),
        ))
    });
    candidates
}

fn surface_rank(
    candidate: &Candidate,
    root_symbols: &BTreeSet<&str>,
    intent_tokens: &BTreeSet<String>,
) -> usize {
    let symbol = candidate.declaration["legacySymbolId"]
        .as_str()
        .unwrap_or_default();
    candidate.score
        + usize::from(root_symbols.contains(symbol)) * 2_000
        + call_declaration_rank(candidate, intent_tokens)
}

fn call_declaration_rank(candidate: &Candidate, intent_tokens: &BTreeSet<String>) -> usize {
    let name = candidate.declaration["name"]
        .as_str()
        .unwrap_or_default()
        .to_lowercase();
    let source = candidate.source_text.to_lowercase();
    let intent_hits = intent_tokens
        .iter()
        .filter(|token| {
            token.len() >= 4 && (name.contains(token.as_str()) || source.contains(token.as_str()))
        })
        .count();
    usize::from(source.contains("@query")) * 700
        + usize::from(source.contains("data class ")) * 250
        + intent_hits * 25
}

fn collect_contracts<'a>(
    _index_facts: &Value,
    selection: &'a TaskContextSelection,
    resolutions: &[Value],
    call_declarations: &[&Candidate],
    support_declarations: &[&Candidate],
) -> Vec<&'a Candidate> {
    let mut type_names = BTreeSet::new();
    for resolution in resolutions {
        for key in ["parameterTypes", "receiverTypes"] {
            type_names.extend(
                resolution
                    .pointer(&format!("/declaration/symbolIdentity/{key}"))
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                    .flat_map(extract_type_names),
            );
        }
        type_names.extend(
            resolution
                .pointer("/declaration/symbolIdentity/returnType")
                .and_then(Value::as_str)
                .into_iter()
                .flat_map(extract_type_names),
        );
        for call in resolution["resolvedCalls"].as_array().into_iter().flatten() {
            type_names.extend(
                ["receiverType", "returnType", "type"]
                    .into_iter()
                    .filter_map(|key| call[key].as_str())
                    .flat_map(extract_type_names),
            );
            if let Some(symbol) = call["symbol"].as_str() {
                let normalized = normalize_symbol(symbol);
                if let Some(owner) = normalized.rsplit('.').nth(1) {
                    type_names.insert(owner.to_owned());
                }
            }
            type_names.extend(
                call["argumentToParameter"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|mapping| mapping["parameterType"].as_str())
                    .flat_map(extract_type_names),
            );
        }
    }
    for candidate in support_declarations {
        for key in ["parameterTypes", "receiverTypes"] {
            type_names.extend(
                candidate
                    .declaration
                    .pointer(&format!("/symbolIdentity/{key}"))
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                    .flat_map(extract_type_names),
            );
        }
        type_names.extend(
            candidate
                .declaration
                .pointer("/symbolIdentity/returnType")
                .and_then(Value::as_str)
                .into_iter()
                .flat_map(extract_type_names),
        );
    }
    let mut contracts = selection
        .catalog
        .iter()
        .filter(|candidate| {
            candidate.declaration["kind"]
                .as_str()
                .is_some_and(|kind| kind.contains("Class"))
                && candidate.declaration["name"]
                    .as_str()
                    .is_some_and(|name| type_names.contains(name))
                && is_contract_source(&candidate.source_text)
        })
        .collect::<Vec<_>>();
    contracts.sort_by_key(|candidate| {
        Reverse(contract_rank(
            candidate,
            &selection.intent_tokens,
            call_declarations,
        ))
    });
    contracts
}

fn collect_projection_fields(
    selection: &TaskContextSelection,
    call_declarations: &[&Candidate],
) -> Vec<Value> {
    let entity_names = call_declarations
        .iter()
        .flat_map(|candidate| {
            let words = candidate.source_text.split_whitespace().collect::<Vec<_>>();
            words
                .windows(2)
                .filter(|window| window[0].eq_ignore_ascii_case("from"))
                .map(|window| {
                    window[1]
                        .trim_matches(|character: char| !character.is_ascii_alphanumeric())
                        .to_owned()
                })
                .collect::<Vec<_>>()
        })
        .collect::<BTreeSet<_>>();
    let mut fields = BTreeMap::<String, Value>::new();
    for candidate in selection.catalog.iter().filter(|candidate| {
        candidate.declaration["kind"]
            .as_str()
            .is_some_and(|kind| kind.contains("Class"))
            && candidate.declaration["name"]
                .as_str()
                .is_some_and(|name| entity_names.contains(name))
    }) {
        let owner = candidate.declaration["name"].as_str().unwrap_or_default();
        for line in candidate.source_text.lines() {
            let trimmed = line.trim();
            let property = ["val ", "var "]
                .into_iter()
                .filter_map(|marker| trimmed.find(marker).map(|index| &trimmed[index + 4..]))
                .next();
            let Some((name, type_and_default)) = property.and_then(|text| text.split_once(':'))
            else {
                continue;
            };
            let name = name.trim();
            if !selection.intent_tokens.contains(&name.to_lowercase()) {
                continue;
            }
            let field_type = type_and_default
                .split(['=', ','])
                .next()
                .unwrap_or_default()
                .trim();
            if field_type.is_empty() {
                continue;
            }
            fields.entry(name.to_owned()).or_insert_with(|| {
                json!({
                    "source":format!("{owner}.{name}"),
                    "type":field_type,
                    "nullable":field_type.ends_with('?')
                })
            });
        }
    }
    fields
        .into_iter()
        .map(|(name, mut field)| {
            field["name"] = json!(name);
            field
        })
        .collect()
}

fn contract_rank(
    candidate: &Candidate,
    intent_tokens: &BTreeSet<String>,
    call_declarations: &[&Candidate],
) -> usize {
    let name = candidate.declaration["name"]
        .as_str()
        .unwrap_or_default()
        .to_lowercase();
    let direct_references = call_declarations
        .iter()
        .filter(|declaration| {
            declaration
                .source_text
                .split(|character: char| !character.is_ascii_alphanumeric() && character != '_')
                .any(|word| word.eq_ignore_ascii_case(&name))
        })
        .count();
    let properties = candidate
        .source_text
        .lines()
        .filter_map(|line| {
            let trimmed = line.trim_start();
            ["val ", "var "]
                .into_iter()
                .filter_map(|marker| trimmed.find(marker).map(|index| &trimmed[index + 4..]))
                .next()
                .and_then(|property| {
                    property
                        .split(|character: char| {
                            !character.is_ascii_alphanumeric() && character != '_'
                        })
                        .next()
                })
                .filter(|property| !property.is_empty())
                .map(str::to_lowercase)
        })
        .collect::<BTreeSet<_>>();
    let property_intent_hits = properties
        .iter()
        .filter(|property| intent_tokens.contains(*property))
        .count();
    let intent_hits = intent_tokens
        .iter()
        .filter(|token| token.len() >= 4 && name.contains(token.as_str()))
        .count();
    candidate.score
        + direct_references * 1_000
        + property_intent_hits * 2_000
        + properties.len().min(8) * 80
        + intent_hits * 100
}

fn collect_execution_path(
    resolutions: &[Value],
    index_facts: &Value,
    selection: &TaskContextSelection,
) -> Vec<Value> {
    let declarations = index_facts["files"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|file| file["declarations"].as_array().into_iter().flatten())
        .collect::<Vec<_>>();
    let mut edges = Vec::new();
    for resolution in resolutions {
        let from = resolution
            .pointer("/declaration/legacySymbolId")
            .and_then(Value::as_str)
            .unwrap_or_default();
        for call in resolution["resolvedCalls"].as_array().into_iter().flatten() {
            let Some(symbol) = call["symbol"].as_str() else {
                continue;
            };
            let normalized = normalize_symbol(symbol);
            let target = declarations.iter().find(|declaration| {
                let legacy =
                    normalize_symbol(declaration["legacySymbolId"].as_str().unwrap_or_default());
                symbol_matches_declaration(&normalized, &legacy, declaration)
            });
            if target.is_none() {
                continue;
            }
            let target = target.expect("checked above");
            let target_candidate = selection.catalog.iter().find(|candidate| {
                candidate.declaration["declarationId"] == target["declarationId"]
            });
            // Syntax-index and K2 declaration ids intentionally use different
            // snapshots. Join them by normalized symbol identity, not by an
            // implementation-specific id, when recognizing resolved roots.
            let target_legacy =
                normalize_symbol(target["legacySymbolId"].as_str().unwrap_or_default());
            let root_target = resolutions.iter().any(|candidate| {
                candidate
                    .pointer("/declaration/legacySymbolId")
                    .and_then(Value::as_str)
                    .map(normalize_symbol)
                    .is_some_and(|root| symbol_matches_declaration(&root, &target_legacy, target))
            });
            let rank = target_candidate.map_or(0, |candidate| {
                call_declaration_rank(candidate, &selection.intent_tokens)
                    + candidate.score
                    + usize::from(root_target) * 900
            });
            edges.push(json!({
                "_rank":rank,
                "from":from.split_once('(').map_or(from,|(prefix,_)|prefix).rsplit('.').next().unwrap_or(from),
                "to":target["name"],
                "file":target["file"],
                "range":[call["start"],call["end"]],
                "returnType":call["returnType"].as_str().map(compact_type),
                "receiverType":call["receiverType"].as_str().map(compact_type),
                "occurrences":1
            }));
        }
    }
    edges.retain(|edge| edge["_rank"].as_u64().unwrap_or_default() >= 300);
    edges.sort_by_key(|edge| {
        (
            Reverse(edge["_rank"].as_u64().unwrap_or_default()),
            edge["from"].as_str().unwrap_or_default().to_owned(),
            edge["range"][0].as_u64().unwrap_or_default(),
        )
    });
    let mut collapsed = Vec::<Value>::new();
    for edge in edges {
        if let Some(existing) = collapsed.iter_mut().find(|existing| {
            existing["from"] == edge["from"]
                && existing["to"] == edge["to"]
                && existing["file"] == edge["file"]
        }) {
            existing["occurrences"] = json!(
                existing["occurrences"].as_u64().unwrap_or(1)
                    + edge["occurrences"].as_u64().unwrap_or(1)
            );
        } else {
            collapsed.push(edge);
        }
    }
    let mut edges = collapsed;
    edges.truncate(MAX_EXECUTION_EDGES);
    for edge in &mut edges {
        edge.as_object_mut()
            .expect("execution edge")
            .remove("_rank");
    }
    edges
}

fn task_needles(
    terms: &[String],
    intent: &str,
    root_candidates: &[&Candidate],
    edit_candidates: &[&Candidate],
    selection: &TaskContextSelection,
) -> BTreeMap<String, usize> {
    let mut needles = BTreeMap::new();
    for token in task_tokens(intent, terms)
        .into_iter()
        .filter(|token| token.len() >= 5)
    {
        needles.insert(token, 3);
    }
    for term in terms
        .iter()
        .filter(|term| !selection.explicit_owners.contains(term.as_str()))
        .filter(|term| {
            !selection
                .files
                .iter()
                .any(|file| exact_test_file_stem(file, term))
        })
    {
        needles.insert(term.clone(), 10);
    }
    for name in edit_candidates
        .iter()
        .filter_map(|candidate| candidate.declaration["name"].as_str())
    {
        needles.insert(name.to_owned(), 12);
    }
    for name in root_candidates
        .iter()
        .filter_map(|candidate| candidate.declaration["name"].as_str())
    {
        needles.insert(name.to_owned(), 40);
    }
    for term in terms.iter().filter(|term| {
        selection
            .files
            .iter()
            .any(|file| exact_test_file_stem(file, term))
    }) {
        // task_tokens also includes the unsplit identifier.  Exact test-file
        // terms are path evidence, never arbitrary body/comment evidence.
        needles.remove(term);
        needles.remove(&term.to_lowercase());
    }
    needles
}

fn collect_anchored_tests(
    selection: &TaskContextSelection,
    needles: &BTreeMap<String, usize>,
    terms: &[String],
) -> Vec<Value> {
    let mut candidates = selection
        .files
        .iter()
        .filter(|file| file.is_test)
        .filter_map(|file| test_snippet(file, needles, terms))
        .collect::<Vec<_>>();
    candidates.sort_by_key(|candidate| Reverse(candidate["score"].as_u64().unwrap_or_default()));
    candidates.truncate(MAX_TESTS);
    candidates
}

fn test_snippet(
    file: &SourceFile,
    needles: &BTreeMap<String, usize>,
    terms: &[String],
) -> Option<Value> {
    let lines = file.source.lines().collect::<Vec<_>>();
    let exact_file_term = terms.iter().find(|term| exact_test_file_stem(file, term));
    let exact_file_score = needles.values().copied().sum::<usize>().saturating_add(1);
    let (function_line, end, score, matched) = lines
        .iter()
        .enumerate()
        .filter(|(_, line)| line.contains("fun "))
        .filter_map(|(function_line, _)| {
            let mut depth = 0isize;
            let mut opened = false;
            let mut end = function_line;
            for (index, line) in lines.iter().enumerate().skip(function_line) {
                depth += line.chars().filter(|character| *character == '{').count() as isize;
                depth -= line.chars().filter(|character| *character == '}').count() as isize;
                opened |= line.contains('{');
                end = index;
                if opened && depth <= 0 {
                    break;
                }
                if index >= function_line + 80 {
                    break;
                }
            }
            let source = lines[function_line..=end.min(lines.len().saturating_sub(1))].join("\n");
            let mut matched = needles
                .iter()
                .filter(|(needle, _)| contains_name(&source, needle))
                .map(|(needle, _)| needle.clone())
                .collect::<Vec<_>>();
            let mut score = matched
                .iter()
                .filter_map(|needle| needles.get(needle))
                .sum::<usize>();
            if let Some(term) = exact_file_term {
                matched.push(term.clone());
                score = score.saturating_add(exact_file_score);
            }
            (!matched.is_empty()).then_some((function_line, end, score, matched))
        })
        .max_by_key(|(function_line, _, score, _)| (*score, Reverse(*function_line)))?;
    let mut start = function_line;
    while start > 0 && start + 4 > function_line {
        let previous = lines[start - 1].trim();
        if previous.starts_with('@') || previous.is_empty() {
            start -= 1;
        } else {
            break;
        }
    }
    let source = lines[start..=end.min(lines.len().saturating_sub(1))].join("\n");
    // PSI declaration.text starts at the first annotation/token, not at the
    // surrounding blank line or its file indentation.
    let declaration_source = source.trim();
    let declaration_name = test_function_name(lines[function_line]).unwrap_or("test");
    let exact_text_hash = canonical::hash_bytes(declaration_source.as_bytes());
    let anchor_id = format!(
        "test-declaration:{}",
        canonical::hash_bytes(
            format!("{}:{declaration_name}:{exact_text_hash}", file.path).as_bytes()
        )
    );
    let owner_occurrences = identifier_occurrences(&file.source, declaration_name);
    let mut snippet = json!({
        "path":file.path,
        "lines":[start+1,end+1],
        "matched":matched,
        "score":score,
        "sourceText":declaration_source
    });
    if owner_occurrences == 1 {
        snippet["declarationTarget"] = json!({
            "anchorId":anchor_id,
            "declarationId":anchor_id,
            "fileId":file.path,
            "ownerSymbolId":declaration_name,
            "syntaxKind":"KtNamedFunction",
            "exactTextHash":exact_text_hash,
            "rangeHint":[start+1,end+1]
        });
    } else {
        snippet["editabilityBoundary"] = json!("AMBIGUOUS_TEST_DECLARATION_OWNER");
        snippet["ownerIdentifierOccurrences"] = json!(owner_occurrences);
    }
    Some(snippet)
}

/// Test sources are not part of the compiler-semantic main index, so their
/// lightweight target currently carries only a PSI declaration name.  Do not
/// advertise that name as an editable owner when the same identifier occurs
/// elsewhere in the file: ApplyEdit would have to guess which declaration it
/// owns. Counting every code/text occurrence is intentionally conservative;
/// false negatives remain usable as read-only test evidence or via CREATE_FILE.
fn identifier_occurrences(source: &str, identifier: &str) -> usize {
    source
        .match_indices(identifier)
        .filter(|(start, _)| {
            let before = source[..*start].chars().next_back();
            let end = start + identifier.len();
            let after = source[end..].chars().next();
            before.is_none_or(|character| !(character.is_alphanumeric() || character == '_'))
                && after.is_none_or(|character| !(character.is_alphanumeric() || character == '_'))
        })
        .count()
}

fn test_function_name(line: &str) -> Option<&str> {
    let declaration = line.split_once("fun ")?.1.trim_start();
    if let Some(rest) = declaration.strip_prefix('`') {
        return rest.split_once('`').map(|(name, _)| name);
    }
    let end = declaration
        .find(|character: char| !(character.is_alphanumeric() || character == '_'))
        .unwrap_or(declaration.len());
    (end > 0).then(|| &declaration[..end])
}

pub fn gradle_test_route(path: &str) -> Option<(String, String)> {
    if path.is_empty() || path.contains('\\') || Path::new(path).is_absolute() {
        return None;
    }
    let components = path.split('/').collect::<Vec<_>>();
    if components
        .iter()
        .any(|component| !canonical_gradle_route_component(component))
    {
        return None;
    }
    let mut contours = components
        .windows(3)
        .enumerate()
        .filter(|(_, window)| *window == ["src", "test", "kotlin"]);
    let (contour, _) = contours.next()?;
    if contours.next().is_some() {
        return None;
    }
    let relative = components.get(contour + 3..)?;
    let file = *relative.last()?;
    let stem = file.strip_suffix(".kt")?;
    if relative.is_empty()
        || stem.is_empty()
        || !stem.ends_with("Test")
        || stem.len() == "Test".len()
        || !valid_kotlin_test_identifier(stem)
    {
        return None;
    }
    let module = &components[..contour];
    let task = if module.is_empty() {
        ":test".to_owned()
    } else {
        format!(":{}:test", module.join(":"))
    };
    Some((task, stem.to_owned()))
}

fn canonical_gradle_route_component(component: &str) -> bool {
    !component.is_empty()
        && !matches!(component, "." | "..")
        && component
            .chars()
            .all(|character| character.is_alphanumeric() || matches!(character, '_' | '-' | '.'))
}

fn valid_kotlin_test_identifier(identifier: &str) -> bool {
    let mut characters = identifier.chars();
    characters
        .next()
        .is_some_and(|character| character == '_' || character.is_alphabetic())
        && characters.all(|character| character == '_' || character.is_alphanumeric())
}

fn authoritative_gradle_module_roots(project: &Value) -> BTreeSet<String> {
    let mut roots = BTreeSet::new();
    if let Some(project_directory) = project.get("projectDirectory").and_then(Value::as_str)
        && let Some(root) = canonical_gradle_module_root(project_directory)
    {
        roots.insert(root);
    }
    for source_root in project
        .get("sourceRoots")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
    {
        if let Some(root) = gradle_module_root_from_main_source(source_root) {
            roots.insert(root);
        }
    }
    roots
}

fn canonical_gradle_module_root(value: &str) -> Option<String> {
    if value.is_empty() || value == "." {
        return Some(String::new());
    }
    if value.contains('\\') || Path::new(value).is_absolute() {
        return None;
    }
    value
        .split('/')
        .all(canonical_gradle_route_component)
        .then(|| value.to_owned())
}

fn gradle_module_root_from_main_source(source_root: &str) -> Option<String> {
    if source_root == "src/main/kotlin" {
        return Some(String::new());
    }
    let module = source_root.strip_suffix("/src/main/kotlin")?;
    canonical_gradle_module_root(module).filter(|root| !root.is_empty())
}

fn authorized_gradle_test_route(
    allowed_roots: &BTreeSet<String>,
    path: &str,
) -> Option<(String, String)> {
    let route = gradle_test_route(path)?;
    let module = if route.0 == ":test" {
        String::new()
    } else {
        route
            .0
            .strip_prefix(':')?
            .strip_suffix(":test")?
            .replace(':', "/")
    };
    allowed_roots.contains(&module).then_some(route)
}

fn validation_plan(project: &Value, tests: &[Value]) -> Value {
    let build_system = project["buildSystem"].as_str().unwrap_or("GRADLE");
    let launcher = project["buildLauncher"]
        .as_str()
        .unwrap_or(match build_system {
            "MAVEN" => "mvn",
            _ => "./gradlew",
        });
    let targeted_args = match build_system {
        "MAVEN" => {
            let stems = tests
                .iter()
                .filter_map(|test| test["path"].as_str())
                .filter_map(|path| Path::new(path).file_stem()?.to_str())
                .collect::<BTreeSet<_>>();
            if stems.is_empty() {
                project_test_tasks(project)
            } else {
                vec![
                    format!("-Dtest={}", stems.into_iter().collect::<Vec<_>>().join(",")),
                    "test".into(),
                ]
            }
        }
        "GRADLE" => {
            let allowed_roots = authoritative_gradle_module_roots(project);
            let mut routes = BTreeMap::<String, BTreeSet<String>>::new();
            for (task, stem) in tests
                .iter()
                .filter_map(|test| test["path"].as_str())
                .filter_map(|path| authorized_gradle_test_route(&allowed_roots, path))
            {
                routes.entry(task).or_default().insert(stem);
            }
            if routes.is_empty() {
                project_test_tasks(project)
            } else {
                let mut args = vec!["cleanTest".into()];
                for (task, stems) in routes {
                    args.push(task);
                    for stem in stems {
                        args.push("--tests".into());
                        args.push(format!("*{stem}"));
                    }
                }
                args
            }
        }
        _ => project_test_tasks(project),
    };
    json!({
        "buildSystem":build_system,
        "buildLauncher":launcher,
        "compileTask":project["compileTask"],
        "targetedArgs":targeted_args,
        "cleanDetachedWorktree":true
    })
}

fn project_test_tasks(project: &Value) -> Vec<String> {
    project["testTasks"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect()
}

fn task_tokens(intent: &str, terms: &[String]) -> BTreeSet<String> {
    let stop = [
        "with",
        "without",
        "from",
        "into",
        "that",
        "this",
        "must",
        "should",
        "change",
        "and",
        "or",
        "the",
        "to",
        "its",
        "для",
        "без",
        "при",
        "или",
        "как",
        "это",
        "нужно",
        "должен",
        "должна",
        "сделать",
    ];
    intent
        .split(|character: char| !(character.is_alphanumeric() || character == '_'))
        .chain(terms.iter().flat_map(|term| split_identifier_tokens(term)))
        .flat_map(token_variants)
        .filter(|token| token.len() >= 2 && !stop.contains(&token.as_str()))
        .collect()
}

fn split_identifier_tokens(value: &str) -> Vec<&str> {
    let mut starts = vec![0];
    let characters = value.char_indices().collect::<Vec<_>>();
    for window in characters.windows(2) {
        let (left_index, left) = window[0];
        let (right_index, right) = window[1];
        if !left.is_alphanumeric() || (left.is_lowercase() && right.is_uppercase()) {
            starts.push(right_index);
        } else if left_index == 0 && !right.is_alphanumeric() {
            starts.push(right_index + right.len_utf8());
        }
    }
    starts.push(value.len());
    starts.sort_unstable();
    starts.dedup();
    let mut tokens = starts
        .windows(2)
        .filter_map(|range| value.get(range[0]..range[1]))
        .map(|token| token.trim_matches(|character: char| !character.is_alphanumeric()))
        .filter(|token| !token.is_empty())
        .collect::<Vec<_>>();
    if tokens.len() > 1 {
        tokens.push(value);
    }
    tokens
}

fn token_variants(value: &str) -> BTreeSet<String> {
    let token = value.to_lowercase();
    let mut variants = BTreeSet::from([token.clone()]);
    if token.len() > 4 && token.ends_with('s') {
        variants.insert(token[..token.len() - 1].to_owned());
    }
    if token.len() > 5 && token.ends_with("ed") && !token.ends_with("eed") {
        variants.insert(token[..token.len() - 1].to_owned());
        variants.insert(token[..token.len() - 2].to_owned());
    }
    if token.len() > 6 && token.ends_with("ing") {
        let stem = &token[..token.len() - 3];
        variants.insert(stem.to_owned());
        if stem.ends_with('v') {
            variants.insert(format!("{stem}e"));
        }
    }
    variants
}

fn extract_type_names(value: &str) -> Vec<String> {
    value
        .split(|character: char| !(character.is_alphanumeric() || character == '_'))
        .filter(|part| part.chars().next().is_some_and(char::is_uppercase))
        .filter(|part| {
            !matches!(
                *part,
                "Any"
                    | "Boolean"
                    | "Collection"
                    | "Int"
                    | "Iterable"
                    | "List"
                    | "Long"
                    | "Map"
                    | "Nothing"
                    | "Sequence"
                    | "Set"
                    | "String"
                    | "Unit"
                    | "UUID"
            )
        })
        .map(str::to_owned)
        .collect()
}

fn normalize_symbol(value: &str) -> String {
    value
        .split_once('(')
        .map_or(value, |(prefix, _)| prefix)
        .replace('/', ".")
}

fn symbol_matches_declaration(symbol: &str, legacy: &str, declaration: &Value) -> bool {
    if symbol == legacy {
        return true;
    }
    let name = declaration["name"].as_str().unwrap_or_default();
    if symbol.ends_with(&format!(".{name}.{name}")) && legacy.ends_with(&format!(".{name}")) {
        return true;
    }
    false
}

fn is_contract_source(source: &str) -> bool {
    let normalized = source.trim_start();
    normalized.starts_with("data class ")
        || normalized.starts_with("interface ")
        || normalized.starts_with("sealed ")
        || normalized.starts_with("enum class ")
        || normalized.starts_with("value class ")
}

fn compact_targets_for_stdout(mut context: Value) -> Value {
    let task = context["task"].as_object_mut().expect("task object");
    for diagnostic in ["intentTokens", "matchedTerms", "unmatchedTerms"] {
        task.remove(diagnostic);
    }
    for (key, prefix) in [("editSurfaces", "S"), ("contracts", "C"), ("tests", "T")] {
        for (index, item) in context
            .get_mut(key)
            .and_then(Value::as_array_mut)
            .into_iter()
            .flatten()
            .enumerate()
        {
            item.as_object_mut()
                .expect("context item")
                .remove("reasons");
            item.as_object_mut().expect("context item").remove("score");
            for (target_key, id_key) in [
                ("declarationTarget", "declarationTargetId"),
                ("bodyTarget", "bodyTargetId"),
            ] {
                let Some(target) = item.get_mut(target_key).map(Value::take) else {
                    continue;
                };
                if target.is_null() {
                    continue;
                }
                let suffix = if target_key == "bodyTarget" { "B" } else { "" };
                let id = json!(format!("{prefix}{}{suffix}", index + 1));
                item[id_key] = id;
                item.as_object_mut()
                    .expect("context item")
                    .remove(target_key);
            }
        }
    }
    for (index, item) in context
        .get_mut("modelInputSurfaces")
        .and_then(Value::as_array_mut)
        .into_iter()
        .flatten()
        .enumerate()
    {
        item["targetId"] = json!(format!("M{}", index + 1));
        item.as_object_mut()
            .expect("model input surface")
            .remove("modelInputTarget");
    }
    for test in context
        .get_mut("tests")
        .and_then(Value::as_array_mut)
        .into_iter()
        .flatten()
    {
        test.as_object_mut().expect("test item").remove("score");
    }
    for edge in context
        .get_mut("executionPath")
        .and_then(Value::as_array_mut)
        .into_iter()
        .flatten()
    {
        edge.as_object_mut()
            .expect("execution edge")
            .retain(|key, _| matches!(key.as_str(), "from" | "to" | "occurrences"));
    }
    context
}

fn compact_type(value: &str) -> String {
    let mut output = String::new();
    let mut token = String::new();
    let flush = |output: &mut String, token: &mut String| {
        if !token.is_empty() {
            output.push_str(token.rsplit('/').next().unwrap_or(token));
            token.clear();
        }
    };
    for character in value.chars() {
        if character.is_alphanumeric() || character == '_' || character == '/' {
            token.push(character);
        } else {
            flush(&mut output, &mut token);
            output.push(character);
        }
    }
    flush(&mut output, &mut token);
    output
}

fn contains_name(haystack: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return false;
    }
    let haystack = haystack.to_lowercase();
    let needle = needle.to_lowercase();
    haystack.match_indices(&needle).any(|(start, _)| {
        let end = start + needle.len();
        let before = haystack[..start].chars().next_back();
        let after = haystack[end..].chars().next();
        before.is_none_or(|character| !(character.is_alphanumeric() || character == '_'))
            && after.is_none_or(|character| !(character.is_alphanumeric() || character == '_'))
    })
}

fn scan_kotlin_sources(repo: &Path) -> Result<Vec<SourceFile>, ClewError> {
    let mut files = WalkDir::new(repo)
        .into_iter()
        .filter_entry(|entry| {
            !matches!(
                entry.file_name().to_str(),
                Some(".git" | ".gradle" | ".semantic-thread" | "build" | "target")
            )
        })
        .filter_map(Result::ok)
        .filter(|entry| {
            entry.file_type().is_file()
                && entry.path().extension().and_then(|value| value.to_str()) == Some("kt")
        })
        .map(|entry| {
            let path = entry.path();
            let relative = path
                .strip_prefix(repo)
                .map_err(|error| ClewError::new(ErrorCode::Internal, error.to_string()))?
                .to_string_lossy()
                .replace('\\', "/");
            let source = std::fs::read_to_string(path)
                .map_err(|error| ClewError::new(ErrorCode::InvalidInput, error.to_string()))?;
            let is_test = relative.starts_with("src/test/") || relative.contains("/src/test/");
            Ok(SourceFile {
                path: relative,
                source,
                is_test,
            })
        })
        .collect::<Result<Vec<_>, ClewError>>()?;
    files.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(files)
}

fn enforce_budget(mut pack: Value, max_bytes: usize) -> Result<Value, ClewError> {
    let original = [
        "editSurfaces",
        "modelInputSurfaces",
        "executionPath",
        "contracts",
        "tests",
    ]
    .into_iter()
    .map(|key| (key.to_owned(), array_len(&pack, key)))
    .collect::<BTreeMap<_, _>>();
    let mut source_bytes = 0usize;
    let mut required_source_bytes = 0usize;
    let mut required_edit_surfaces_omitted = 0usize;
    let content_budget = max_bytes.saturating_sub(256).max(3_072);
    while serialized_len(&pack)? > content_budget {
        if array_len(&pack, "executionPath") > 6 && pop_array(&mut pack, "executionPath") {
            continue;
        }
        if array_len(&pack, "tests") > 1 && pop_array(&mut pack, "tests") {
            continue;
        }
        if array_len(&pack, "contracts") > 2 && pop_array(&mut pack, "contracts") {
            continue;
        }
        if let Some(omitted) = strip_largest_optional_source(&mut pack) {
            source_bytes += omitted;
            continue;
        }
        // Optional graph expansion is useful orientation, not part of the
        // required edit surface. Remove it before degrading an entrypoint,
        // explicit target, contract, or requested test.
        if pop_last_optional(&mut pack, "editSurfaces") {
            continue;
        }
        if let Some((omitted, required)) = trim_largest_source(&mut pack) {
            source_bytes += omitted;
            if required {
                required_source_bytes += omitted;
            }
            continue;
        }
        if array_len(&pack, "editSurfaces") > 1 {
            let required = pack["editSurfaces"]
                .as_array()
                .and_then(|items| items.last())
                .and_then(|item| item["required"].as_bool())
                == Some(true);
            if pop_array(&mut pack, "editSurfaces") {
                required_edit_surfaces_omitted += usize::from(required);
                continue;
            }
        }
        return Err(ClewError::new(
            ErrorCode::InvalidInput,
            format!("--max-bytes {max_bytes} is too small for the mandatory task closure"),
        ));
    }
    let omitted = original
        .iter()
        .map(|(key, count)| (key.clone(), count - array_len(&pack, key)))
        .collect::<BTreeMap<_, _>>();
    let partial = required_source_bytes > 0
        || required_edit_surfaces_omitted > 0
        || omitted
            .iter()
            .any(|(key, count)| key != "editSurfaces" && *count > 0);
    if partial {
        pack["completeness"]["status"] = json!("PARTIAL_TASK");
        pack["completeness"]["boundaries"]
            .as_array_mut()
            .expect("boundaries array")
            .push(json!({"kind":"STDOUT_BUDGET","omitted":omitted,"sourceBytes":source_bytes}));
    }
    pack["completeness"]["omitted"] = json!({
        "editSurfaces":omitted.get("editSurfaces").copied().unwrap_or_default(),
        "modelInputSurfaces":omitted.get("modelInputSurfaces").copied().unwrap_or_default(),
        "executionPath":omitted.get("executionPath").copied().unwrap_or_default(),
        "contracts":omitted.get("contracts").copied().unwrap_or_default(),
        "tests":omitted.get("tests").copied().unwrap_or_default(),
        "sourceBytes":source_bytes
    });
    if serialized_len(&pack)? > max_bytes {
        return Err(ClewError::new(
            ErrorCode::Internal,
            "task context budget accounting failed",
        ));
    }
    Ok(pack)
}

fn pop_last_optional(value: &mut Value, key: &str) -> bool {
    let Some(items) = value.get_mut(key).and_then(Value::as_array_mut) else {
        return false;
    };
    let Some(index) = items
        .iter()
        .rposition(|item| item["surfaceRequired"].as_bool() == Some(false))
    else {
        return false;
    };
    items.remove(index);
    true
}

fn strip_largest_optional_source(pack: &mut Value) -> Option<usize> {
    let (index, length) = pack["editSurfaces"]
        .as_array()?
        .iter()
        .enumerate()
        .filter(|(_, item)| item["required"].as_bool() == Some(false))
        .filter_map(|(index, item)| Some((index, item["sourceText"].as_str()?.len())))
        .filter(|(_, length)| *length > 0)
        .max_by_key(|(_, length)| *length)?;
    let item = pack["editSurfaces"].as_array_mut()?.get_mut(index)?;
    item["sourceText"] = json!("");
    item["sourceProjection"] = json!("GRAPH_SIGNATURE_ONLY");
    item["sourceBytesOmitted"] = json!(length);
    Some(length)
}

fn trim_largest_source(pack: &mut Value) -> Option<(usize, bool)> {
    let mut best: Option<(String, usize, usize, bool)> = None;
    for key in ["editSurfaces", "contracts", "tests"] {
        for (index, item) in pack.get(key)?.as_array()?.iter().enumerate() {
            let length = item["sourceText"]
                .as_str()
                .map(str::len)
                .unwrap_or_default();
            let required = item["required"].as_bool() == Some(true);
            if length > 640
                && best
                    .as_ref()
                    .is_none_or(|(_, _, best_length, best_required)| {
                        (!required && *best_required)
                            || (required == *best_required && length > *best_length)
                    })
            {
                best = Some((key.to_owned(), index, length, required));
            }
        }
    }
    let (key, index, length, required) = best?;
    let item = pack.get_mut(&key)?.as_array_mut()?.get_mut(index)?;
    let source = item["sourceText"].as_str()?.to_owned();
    let (truncated, _, omitted) = truncate_utf8(&source, (length / 2).max(640));
    item["sourceText"] = json!(truncated);
    item["sourceTruncated"] = json!(true);
    let previous = item["sourceBytesOmitted"].as_u64().unwrap_or_default() as usize;
    item["sourceBytesOmitted"] = json!(previous + omitted);
    Some((omitted, required))
}

fn serialized_len(value: &Value) -> Result<usize, ClewError> {
    canonical::pretty(value)
        .map(|text| text.len() + 1)
        .map_err(|error| ClewError::new(ErrorCode::Internal, error.to_string()))
}

fn array_len(value: &Value, key: &str) -> usize {
    value[key].as_array().map_or(0, Vec::len)
}
fn pop_array(value: &mut Value, key: &str) -> bool {
    value
        .get_mut(key)
        .and_then(Value::as_array_mut)
        .and_then(Vec::pop)
        .is_some()
}

fn truncate_utf8(value: &str, max_bytes: usize) -> (String, bool, usize) {
    if value.len() <= max_bytes {
        return (value.to_owned(), false, 0);
    }
    let mut end = max_bytes.saturating_sub(3).min(value.len());
    while end > 0 && !value.is_char_boundary(end) {
        end -= 1;
    }
    (format!("{}...", &value[..end]), true, value.len() - end)
}

fn utf16_range_to_bytes(source: &str, start: usize, end: usize) -> (usize, usize) {
    (
        utf16_offset_to_byte(source, start),
        utf16_offset_to_byte(source, end),
    )
}
fn utf16_offset_to_byte(source: &str, target: usize) -> usize {
    let mut units = 0;
    for (byte, character) in source.char_indices() {
        if units >= target {
            return byte;
        }
        units += character.len_utf16();
    }
    source.len()
}
fn line_number(source: &str, byte_offset: usize) -> usize {
    source
        .get(..byte_offset.min(source.len()))
        .unwrap_or_default()
        .bytes()
        .filter(|byte| *byte == b'\n')
        .count()
        + 1
}

fn evidence_display(repo: &Path, evidence_path: &Path) -> String {
    let absolute = if evidence_path.is_absolute() {
        PathBuf::from(evidence_path)
    } else {
        repo.join(evidence_path)
    };
    absolute
        .strip_prefix(repo)
        .unwrap_or(&absolute)
        .to_string_lossy()
        .replace('\\', "/")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounded_context_keeps_a_five_declaration_hook_closure() {
        assert!(MAX_EDIT_SURFACES >= 5);
        assert_eq!(MAX_EDIT_SURFACES, 8);
    }

    fn initialize_git_repo() -> tempfile::TempDir {
        let temporary = tempfile::tempdir().unwrap();
        assert!(
            Command::new("git")
                .args(["init", "--quiet", "--initial-branch=main"])
                .current_dir(temporary.path())
                .status()
                .unwrap()
                .success()
        );
        temporary
    }

    fn project_with_model_inputs(entries: Vec<Value>) -> Value {
        let manifest = json!({
            "schema":"kotlin-semantic-input-manifest/0.1",
            "orderedCompileClasspath":[],
            "modelInputs":entries,
        });
        json!({
            "projectModelHash":"sha256:project",
            "semanticInputManifestHash":canonical::hash(&manifest).unwrap(),
            "semanticInputManifest":manifest,
        })
    }

    fn track(repo: &Path, path: &str, bytes: &[u8]) {
        let absolute = repo.join(path);
        std::fs::create_dir_all(absolute.parent().unwrap()).unwrap();
        std::fs::write(&absolute, bytes).unwrap();
        assert!(
            Command::new("git")
                .args(["add", "--", path])
                .current_dir(repo)
                .status()
                .unwrap()
                .success()
        );
    }

    #[test]
    fn model_input_surface_is_exact_required_and_untruncated() {
        let temporary = initialize_git_repo();
        let source = format!(
            "plugins {{\n    kotlin(\"jvm\")\n}}\n{}",
            "// full\n".repeat(900)
        );
        track(
            temporary.path(),
            "workers/kotlin21/build.gradle.kts",
            source.as_bytes(),
        );
        let exact_hash = canonical::hash_bytes(source.as_bytes());
        let project = project_with_model_inputs(vec![json!({
            "path":"workers/kotlin21/build.gradle.kts",
            "hash":exact_hash,
        })]);

        let surfaces = resolve_model_input_surfaces(
            temporary.path(),
            &project,
            &["workers/kotlin21/build.gradle.kts".into()],
        )
        .unwrap();

        assert_eq!(surfaces.len(), 1);
        assert_eq!(surfaces[0]["targetId"], "M1");
        assert_eq!(surfaces[0]["status"], "REQUIRED");
        assert_eq!(surfaces[0]["required"], true);
        assert_eq!(surfaces[0]["exactHash"], exact_hash);
        assert_eq!(surfaces[0]["sourceText"], source);
        assert!(surfaces[0].get("sourceTruncated").is_none());
        assert_eq!(
            surfaces[0]["modelInputTarget"]["semanticInputManifestHash"],
            project["semanticInputManifestHash"]
        );

        let compact = compact_targets_for_stdout(json!({
            "task":{},
            "modelInputSurfaces":surfaces,
            "editSurfaces":[],
            "contracts":[],
            "tests":[],
            "executionPath":[],
        }));
        assert_eq!(compact["modelInputSurfaces"][0]["targetId"], "M1");
        assert_eq!(compact["modelInputSurfaces"][0]["sourceText"], source);
        assert!(
            compact["modelInputSurfaces"][0]
                .get("modelInputTarget")
                .is_none()
        );

        let bounded = enforce_budget(
            json!({
                "task":{},
                "modelInputSurfaces":compact["modelInputSurfaces"],
                "editSurfaces":[],
                "contracts":[],
                "tests":[],
                "executionPath":[],
                "completeness":{"status":"COMPLETE_TASK","boundaries":[],"omitted":{}},
            }),
            16_384,
        )
        .unwrap();
        assert_eq!(bounded["completeness"]["status"], "COMPLETE_TASK");
        assert_eq!(bounded["modelInputSurfaces"][0]["sourceText"], source);
        assert_eq!(bounded["completeness"]["omitted"]["modelInputSurfaces"], 0);

        let too_small = enforce_budget(
            json!({
                "task":{},
                "modelInputSurfaces":bounded["modelInputSurfaces"],
                "editSurfaces":[],
                "contracts":[],
                "tests":[],
                "executionPath":[],
                "completeness":{"status":"COMPLETE_TASK","boundaries":[],"omitted":{}},
            }),
            4_096,
        );
        assert!(too_small.is_err());
    }

    #[test]
    fn model_input_surface_rejects_untracked_noncanonical_and_wrong_digest_paths() {
        let temporary = initialize_git_repo();
        std::fs::write(
            temporary.path().join("untracked.gradle.kts"),
            "plugins {}\n",
        )
        .unwrap();
        track(
            temporary.path(),
            "settings.gradle.kts",
            b"rootProject.name = \"demo\"\n",
        );
        let settings_hash = canonical::hash_bytes(b"rootProject.name = \"demo\"\n");
        let project = project_with_model_inputs(vec![
            json!({"path":"untracked.gradle.kts","hash":canonical::hash_bytes(b"plugins {}\n")}),
            json!({"path":"settings.gradle.kts","hash":"sha256:wrong"}),
        ]);

        for requested in [
            "untracked.gradle.kts",
            "./settings.gradle.kts",
            "settings.gradle.kts",
        ] {
            assert!(
                resolve_model_input_surfaces(temporary.path(), &project, &[requested.into()])
                    .is_err(),
                "{requested} must be refused"
            );
        }
        let valid_project = project_with_model_inputs(vec![
            json!({"path":"settings.gradle.kts","hash":settings_hash}),
        ]);
        assert!(
            resolve_model_input_surfaces(
                temporary.path(),
                &valid_project,
                &["settings.gradle.kts".into(), "settings.gradle.kts".into()],
            )
            .is_err()
        );
    }

    #[cfg(unix)]
    #[test]
    fn model_input_surface_rejects_a_tracked_symlink() {
        use std::os::unix::fs::symlink;

        let temporary = initialize_git_repo();
        std::fs::write(temporary.path().join("actual.gradle.kts"), "plugins {}\n").unwrap();
        symlink(
            "actual.gradle.kts",
            temporary.path().join("linked.gradle.kts"),
        )
        .unwrap();
        assert!(
            Command::new("git")
                .args(["add", "--", "linked.gradle.kts"])
                .current_dir(temporary.path())
                .status()
                .unwrap()
                .success()
        );
        let project = project_with_model_inputs(vec![json!({
            "path":"linked.gradle.kts",
            "hash":canonical::hash_bytes(b"plugins {}\n"),
        })]);

        assert!(
            resolve_model_input_surfaces(
                temporary.path(),
                &project,
                &["linked.gradle.kts".into()],
            )
            .is_err()
        );
    }

    fn function_candidate(name: &str) -> Candidate {
        Candidate {
            declaration: json!({
                "name":name,
                "kind":"KtNamedFunction",
                "legacySymbolId":format!("com.acme.Service.{name}"),
                "declarationId":format!("declaration:{name}"),
                "file":"src/main/kotlin/com/acme/Service.kt"
            }),
            source_text: format!("fun {name}() = Unit"),
            line_start: 1,
            line_end: 1,
            score: 0,
            reasons: BTreeSet::new(),
        }
    }

    fn selection(catalog: Vec<Candidate>) -> TaskContextSelection {
        TaskContextSelection {
            files: Vec::new(),
            catalog,
            intent_tokens: BTreeSet::new(),
            goal_tokens: BTreeSet::new(),
            explicit_owners: BTreeSet::new(),
            requirements: Vec::new(),
            required_candidate_ids: BTreeSet::new(),
            entrypoint_candidate_ids: BTreeSet::new(),
            explicit_candidate_ids: BTreeSet::new(),
            requires_tests: false,
        }
    }

    #[test]
    fn intent_tokens_split_camel_case_and_keep_payload_fields() {
        let tokens = task_tokens(
            "synchronize typed key/label payload",
            &["RecordService".into()],
        );
        assert!(tokens.contains("synchronize"));
        assert!(tokens.contains("key"));
        assert!(tokens.contains("typed"));
    }

    #[test]
    fn plausible_terms_are_split_and_inflected_intent_is_normalized() {
        let tokens = task_tokens(
            "When processing records",
            &["processRecord".into(), "RecordDeltaStream".into()],
        );
        assert!(tokens.contains("process"));
        assert!(tokens.contains("record"));
        assert!(tokens.contains("stream"));
    }

    #[test]
    fn enum_literal_is_not_treated_as_an_exact_declaration_name() {
        assert!(looks_like_constant("APPLIED"));
        assert!(looks_like_constant("ROW_APPLIED"));
        assert!(!looks_like_constant("processRecord"));
    }

    #[test]
    fn named_test_file_is_a_test_surface_without_a_main_edit_surface() {
        let files = vec![SourceFile {
            path: "src/test/kotlin/com/acme/ProjectModelCommandTest.kt".into(),
            source: "class ProjectModelCommandTest".into(),
            is_test: true,
        }];

        let (requirements, required, entrypoints, explicit) =
            derive_requirements(&files, &[], &["ProjectModelCommandTest".into()]);

        assert_eq!(requirements.len(), 1);
        assert_eq!(requirements[0].kind, "TEST_SURFACE");
        assert!(requirements[0].satisfied);
        assert_eq!(
            requirements[0].evidence_paths,
            vec!["src/test/kotlin/com/acme/ProjectModelCommandTest.kt"]
        );
        assert!(required.is_empty());
        assert!(entrypoints.is_empty());
        assert!(explicit.is_empty());
    }

    #[test]
    fn compact_surface_preserves_full_required_source_before_budgeting() {
        let mut candidate = function_candidate("largeTarget");
        candidate.source_text = "x".repeat(30_000);

        let surface = compact_surface(&candidate, None);

        assert_eq!(surface["sourceText"].as_str().unwrap().len(), 30_000);
        assert!(surface.get("sourceTruncated").is_none());
        assert!(surface.get("sourceBytesOmitted").is_none());
    }

    #[test]
    fn exact_test_declaration_is_not_a_production_edit_surface() {
        let files = vec![SourceFile {
            path: "src/test/kotlin/com/acme/ProjectModelCommandTest.kt".into(),
            source: "class ProjectModelCommandTest".into(),
            is_test: true,
        }];
        let mut test_candidate = function_candidate("ProjectModelCommandTest");
        test_candidate.declaration["file"] =
            json!("src/test/kotlin/com/acme/ProjectModelCommandTest.kt");

        let (requirements, required, _, _) = derive_requirements(
            &files,
            &[test_candidate],
            &["ProjectModelCommandTest".into()],
        );

        assert_eq!(requirements[0].kind, "TEST_SURFACE");
        assert!(requirements[0].satisfied);
        assert!(required.is_empty());
    }

    #[test]
    fn exact_test_function_is_a_test_surface_without_a_production_edit_surface() {
        let path = "src/test/kotlin/com/acme/ProjectModelCommandTest.kt";
        let files = vec![SourceFile {
            path: path.into(),
            source: "class ProjectModelCommandTest {\n    @Test\n    fun gradlePlanIsOfflineAndUsesOnlyRepoOwnedHome() = Unit\n}".into(),
            is_test: true,
        }];

        let (requirements, required, _, explicit) = derive_requirements(
            &files,
            &[],
            &["gradlePlanIsOfflineAndUsesOnlyRepoOwnedHome".into()],
        );

        assert_eq!(requirements[0].kind, "TEST_SURFACE");
        assert!(requirements[0].satisfied);
        assert_eq!(requirements[0].evidence_paths, vec![path]);
        assert!(required.is_empty());
        assert!(explicit.is_empty());
        assert!(requirement_is_satisfied(
            &requirements[0],
            &[json!({"path":path})]
        ));
        assert!(!requirement_is_satisfied(
            &requirements[0],
            &[json!({"path":"src/test/kotlin/com/acme/OtherTest.kt"})]
        ));
    }

    #[test]
    fn exact_object_declaration_emits_a_required_immutable_target() {
        let path =
            "workers/kotlin21/src/main/kotlin/dev/semanticthread/worker/BtaIncrementalBackend21.kt";
        let source = "private object RealBtaCompilation21 : BtaCompilation21";
        let files = vec![SourceFile {
            path: path.into(),
            source: source.into(),
            is_test: false,
        }];
        let mut object = function_candidate("RealBtaCompilation21");
        object.declaration["kind"] = json!("KtObjectDeclaration");
        object.declaration["file"] = json!(path);
        object.declaration["symbolId"] = json!("dev.semanticthread.worker.RealBtaCompilation21");
        object.declaration["rangeStart"] = json!(0);
        object.declaration["rangeEnd"] = json!(source.len());
        object.source_text = source.into();

        let (requirements, required, _, explicit) =
            derive_requirements(&files, &[object.clone()], &["RealBtaCompilation21".into()]);
        let surface = compact_surface(&object, None);

        assert_eq!(requirements[0].kind, "EXPLICIT_DECLARATION");
        assert!(requirements[0].satisfied);
        assert_eq!(
            required,
            BTreeSet::from(["declaration:RealBtaCompilation21".into()])
        );
        assert_eq!(required, explicit);
        assert_eq!(surface["declarationTarget"]["fileId"], path);
        assert_eq!(
            surface["declarationTarget"]["syntaxKind"],
            "KtObjectDeclaration"
        );
        assert_eq!(
            surface["declarationTarget"]["exactTextHash"],
            canonical::hash_bytes(source.as_bytes())
        );

        let mut property = object;
        property.declaration["kind"] = json!("KtProperty");
        assert!(!is_surface_declaration(&property));
    }

    #[test]
    fn unanchored_evidence_term_in_a_comment_remains_unsatisfied() {
        let files = vec![SourceFile {
            path: "src/main/kotlin/com/acme/Service.kt".into(),
            source: "// MissingSurface is not an immutable target".into(),
            is_test: false,
        }];

        let (requirements, required, _, _) =
            derive_requirements(&files, &[], &["MissingSurface".into()]);

        assert_eq!(requirements[0].kind, "EVIDENCE_TERM");
        assert!(!requirements[0].satisfied);
        assert!(requirements[0].candidate_ids.is_empty());
        assert!(requirements[0].evidence_paths.is_empty());
        assert!(required.is_empty());
    }

    #[test]
    fn same_named_main_and_test_files_select_the_main_surface() {
        let files = vec![
            SourceFile {
                path: "src/main/kotlin/com/acme/Runner.kt".into(),
                source: "fun main() = Unit".into(),
                is_test: false,
            },
            SourceFile {
                path: "src/test/kotlin/com/acme/Runner.kt".into(),
                source: "class Runner".into(),
                is_test: true,
            },
        ];
        let mut main = function_candidate("main");
        main.declaration["file"] = json!("src/main/kotlin/com/acme/Runner.kt");

        let (requirements, required, _, _) =
            derive_requirements(&files, &[main], &["Runner".into()]);

        assert_eq!(requirements[0].kind, "ENTRYPOINT_FILE");
        assert!(requirements[0].satisfied);
        assert_eq!(required, BTreeSet::from(["declaration:main".into()]));
    }

    #[test]
    fn required_source_that_fits_global_budget_remains_complete() {
        let source = "r".repeat(30_000);
        let pack = json!({
            "task":{},
            "editSurfaces":[{"name":"main","required":true,"surfaceRequired":true,"sourceText":source}],
            "executionPath":[],
            "contracts":[],
            "tests":[],
            "completeness":{"status":"COMPLETE_TASK","boundaries":[],"omitted":{}}
        });

        let bounded = enforce_budget(pack, 65_536).unwrap();

        assert_eq!(bounded["completeness"]["status"], "COMPLETE_TASK");
        assert_eq!(
            bounded["editSurfaces"][0]["sourceText"]
                .as_str()
                .unwrap()
                .len(),
            30_000
        );
        assert_eq!(bounded["completeness"]["omitted"]["sourceBytes"], 0);
    }

    #[test]
    fn required_test_source_is_complete_before_global_budgeting() {
        let body = "x".repeat(8_000);
        let files = SourceFile {
            path: "src/test/kotlin/com/acme/LargeTest.kt".into(),
            source: format!("@Test\nfun verifiesTarget() {{\n// target\n{body}\n}}"),
            is_test: true,
        };
        let needles = BTreeMap::from([("target".into(), 1)]);

        let test = test_snippet(&files, &needles, &[]).unwrap();

        assert!(test["sourceText"].as_str().unwrap().len() > 8_000);
        assert!(test.get("sourceTruncated").is_none());
        assert!(test.get("sourceBytesOmitted").is_none());
    }

    #[test]
    fn ambiguous_test_owner_is_read_only_before_task_apply() {
        let files = SourceFile {
            path: "src/test/kotlin/com/acme/CacheTest.kt".into(),
            source: "class CacheTest {\n    private fun model() = Unit\n    @Test fun verifies() { val model = 1; check(model == 1) }\n}".into(),
            is_test: true,
        };
        let needles = BTreeMap::from([("model".into(), 1)]);

        let test = test_snippet(&files, &needles, &[]).unwrap();

        assert!(test.get("declarationTarget").is_none());
        assert_eq!(
            test["editabilityBoundary"],
            "AMBIGUOUS_TEST_DECLARATION_OWNER"
        );
        assert_eq!(test["ownerIdentifierOccurrences"], 3);
    }

    #[test]
    fn unique_test_owner_retains_exact_edit_target() {
        let files = SourceFile {
            path: "src/test/kotlin/com/acme/CacheTest.kt".into(),
            source: "class CacheTest {\n    @Test fun publishesManifest() = Unit\n}".into(),
            is_test: true,
        };
        let needles = BTreeMap::from([("publishesManifest".into(), 1)]);

        let test = test_snippet(&files, &needles, &[]).unwrap();

        assert_eq!(
            test["declarationTarget"]["ownerSymbolId"],
            "publishesManifest"
        );
        assert!(test.get("editabilityBoundary").is_none());
    }

    #[test]
    fn named_file_entrypoint_and_disconnected_declarations_are_all_required() {
        let mut boot = function_candidate("main");
        boot.declaration["declarationId"] = json!("declaration:boot");
        boot.declaration["legacySymbolId"] = json!("com.acme.Runner.main");
        boot.declaration["file"] = json!("src/main/kotlin/com/acme/Runner.kt");
        boot.score = 1;
        let mut read_options = function_candidate("readOptions");
        read_options.score = 500;
        let mut apply_options = function_candidate("applyOptions");
        apply_options.score = 400;
        let files = vec![
            SourceFile {
                path: "src/main/kotlin/com/acme/Runner.kt".into(),
                source: "fun main() = Unit".into(),
                is_test: false,
            },
            SourceFile {
                path: "src/main/kotlin/com/acme/Service.kt".into(),
                source: "fun readOptions() = Unit\nfun applyOptions() = Unit".into(),
                is_test: false,
            },
        ];
        let catalog = vec![boot, read_options, apply_options];
        let (requirements, required, entrypoints, explicit) = derive_requirements(
            &files,
            &catalog,
            &["Runner".into(), "readOptions".into(), "applyOptions".into()],
        );
        let mut selected = selection(catalog);
        selected.requirements = requirements;
        selected.required_candidate_ids = required;
        selected.entrypoint_candidate_ids = entrypoints;
        selected.explicit_candidate_ids = explicit;

        assert_eq!(selected.root_symbols(1), vec!["com.acme.Runner.main"]);
        assert_eq!(selected.required_candidate_ids.len(), 3);
        assert!(selected.requirements.iter().all(|item| item.satisfied));
    }

    #[test]
    fn test_request_is_language_agnostic_and_explicit() {
        assert!(intent_requests_tests("add three focused tests"));
        assert!(intent_requests_tests("добавить регрессионные тесты"));
        assert!(intent_requests_tests("use human-readable @DisplayName"));
        assert!(!intent_requests_tests("preserve runtime behavior"));
    }

    #[test]
    fn gradle_targeted_validation_runs_test_task_before_filters() {
        let plan = validation_plan(
            &json!({
                "buildSystem":"GRADLE",
                "buildLauncher":"./gradlew",
                "compileTask":":compileKotlin",
                "projectDirectory":"",
                "sourceRoots":["src/main/kotlin"]
            }),
            &[json!({"path":"src/test/kotlin/com/acme/RunnerTest.kt"})],
        );

        assert_eq!(
            plan["targetedArgs"],
            json!(["cleanTest", ":test", "--tests", "*RunnerTest"])
        );
    }

    #[test]
    fn gradle_targeted_validation_routes_k21_test_surface_to_owning_module() {
        let plan = validation_plan(
            &json!({
                "buildSystem":"GRADLE",
                "buildLauncher":"./gradlew",
                "projectPath":":workers:kotlin21",
                "compileTask":":workers:kotlin21:compileKotlin",
                "projectDirectory":"workers/kotlin21",
                "sourceRoots":[
                    "workers/kotlin/src/main/kotlin",
                    "workers/kotlin21/src/main/kotlin"
                ]
            }),
            &[json!({
                "path":"workers/kotlin21/src/test/kotlin/dev/semanticthread/worker/K2FactGenerationStore21Test.kt"
            })],
        );

        assert_eq!(
            plan["targetedArgs"],
            json!([
                "cleanTest",
                ":workers:kotlin21:test",
                "--tests",
                "*K2FactGenerationStore21Test"
            ])
        );
        assert!(
            !plan["targetedArgs"]
                .as_array()
                .unwrap()
                .iter()
                .any(|argument| argument == "test")
        );
    }

    #[test]
    fn exact_test_stem_outranks_unrelated_generic_evidence_and_routes_its_module() {
        let term = "BtaIncrementalBackend21Test".to_owned();
        let root = function_candidate("RealBtaCompilation21");
        let mut selection = selection(Vec::new());
        selection.files = vec![
            SourceFile {
                path: "workers/kotlin/src/test/kotlin/dev/semanticthread/worker/ProjectModelCommandTest.kt".into(),
                source: "@Test\nfun genericModelTest() {\n    // BtaIncrementalBackend21Test is only a comment\n    RealBtaCompilation21\n}".into(),
                is_test: true,
            },
            SourceFile {
                path: "workers/kotlin21/src/test/kotlin/dev/semanticthread/worker/BtaIncrementalBackend21Test.kt".into(),
                source: "@Test\nfun compilesThroughBta() = Unit".into(),
                is_test: true,
            },
        ];
        let terms = vec![term.clone()];
        let needles = task_needles(
            &terms,
            "add focused regression tests",
            &[&root],
            &[&root],
            &selection,
        );

        assert!(!needles.contains_key(&term));
        assert!(!needles.contains_key(&term.to_lowercase()));
        let tests = collect_anchored_tests(&selection, &needles, &terms);
        assert_eq!(tests.len(), 1);
        assert_eq!(
            tests[0]["path"],
            "workers/kotlin21/src/test/kotlin/dev/semanticthread/worker/BtaIncrementalBackend21Test.kt"
        );
        assert!(
            tests[0]["matched"]
                .as_array()
                .unwrap()
                .iter()
                .any(|matched| matched == &term)
        );
        let (requirements, _, _, _) = derive_requirements(&selection.files, &[], &terms);
        assert!(requirement_is_satisfied(&requirements[0], &tests));
        assert!(!requirement_is_satisfied(
            &requirements[0],
            &[json!({
                "path":"workers/kotlin/src/test/kotlin/dev/semanticthread/worker/ProjectModelCommandTest.kt"
            })],
        ));

        let plan = validation_plan(
            &json!({
                "buildSystem":"GRADLE",
                "buildLauncher":"./gradlew",
                "compileTask":":workers:kotlin21:compileKotlin",
                "projectDirectory":"workers/kotlin21",
                "sourceRoots":["workers/kotlin21/src/main/kotlin"]
            }),
            &tests,
        );
        assert_eq!(
            plan["targetedArgs"],
            json!([
                "cleanTest",
                ":workers:kotlin21:test",
                "--tests",
                "*BtaIncrementalBackend21Test"
            ])
        );
    }

    #[test]
    fn gradle_targeted_validation_keeps_each_module_filter_with_its_owner() {
        let plan = validation_plan(
            &json!({
                "buildSystem":"GRADLE",
                "buildLauncher":"./gradlew",
                "compileTask":":workers:kotlin21:compileKotlin",
                "projectDirectory":"workers/kotlin21",
                "sourceRoots":["workers/kotlin/src/main/kotlin"]
            }),
            &[
                json!({
                    "path":"workers/kotlin/src/test/kotlin/dev/semanticthread/worker/CommonContractTest.kt"
                }),
                json!({
                    "path":"workers/kotlin21/src/test/kotlin/dev/semanticthread/worker/K2FactGenerationStore21Test.kt"
                }),
            ],
        );

        assert_eq!(
            plan["targetedArgs"],
            json!([
                "cleanTest",
                ":workers:kotlin21:test",
                "--tests",
                "*K2FactGenerationStore21Test",
                ":workers:kotlin:test",
                "--tests",
                "*CommonContractTest"
            ])
        );
    }

    #[test]
    fn k21_validation_excludes_standalone_fixture_test_surface() {
        let plan = validation_plan(
            &json!({
                "buildSystem":"GRADLE",
                "buildLauncher":"./gradlew",
                "compileTask":":workers:kotlin21:compileKotlin",
                "testTasks":["cleanTest","test"],
                "projectDirectory":"workers/kotlin21",
                "sourceRoots":[
                    "workers/kotlin/src/main/kotlin",
                    "workers/kotlin21/src/main/kotlin"
                ]
            }),
            &[json!({
                "path":"fixtures/kotlin-2-1/src/test/kotlin/example/RunnerTest.kt"
            })],
        );

        assert_eq!(plan["targetedArgs"], json!(["cleanTest", "test"]));
    }

    #[test]
    fn gradle_targeted_validation_rejects_paths_outside_or_tricking_model_roots() {
        let project = json!({
            "buildSystem":"GRADLE",
            "buildLauncher":"./gradlew",
            "compileTask":":workers:kotlin21:compileKotlin",
            "testTasks":["test"],
            "projectDirectory":"workers/kotlin21",
            "sourceRoots":["workers/kotlin/src/main/kotlin"]
        });
        for path in [
            "fixtures/kotlin-2-1/src/test/kotlin/example/RunnerTest.kt",
            "workers/kotlin21/../kotlin/src/test/kotlin/example/RunnerTest.kt",
            "/workers/kotlin21/src/test/kotlin/example/RunnerTest.kt",
            "workers\\kotlin21\\src\\test\\kotlin\\example\\RunnerTest.kt",
        ] {
            let plan = validation_plan(&project, &[json!({"path":path})]);
            assert_eq!(plan["targetedArgs"], json!(["test"]), "path={path}");
        }
    }

    #[test]
    fn authoritative_gradle_roots_are_only_project_directory_and_main_source_owners() {
        let roots = authoritative_gradle_module_roots(&json!({
            "projectDirectory":"workers/kotlin21",
            "sourceRoots":[
                "workers/kotlin/src/main/kotlin",
                "src/main/kotlin",
                "fixtures/kotlin-2-1/src/test/kotlin",
                "workers/kotlin21/../kotlin/src/main/kotlin",
                "/absolute/src/main/kotlin"
            ]
        }));

        assert_eq!(
            roots,
            BTreeSet::from([
                String::new(),
                "workers/kotlin".to_owned(),
                "workers/kotlin21".to_owned(),
            ])
        );
    }

    #[test]
    fn stdout_budget_keeps_optional_graph_role_before_required_source() {
        let required_source = "r".repeat(2_000);
        let optional_source = "o".repeat(2_000);
        let pack = json!({
            "task":{},
            "editSurfaces":[
                {"name":"main","required":true,"surfaceRequired":true,"sourceText":required_source},
                {"name":"loadRecords","required":false,"surfaceRequired":true,"sourceText":optional_source}
            ],
            "executionPath":[],
            "contracts":[],
            "tests":[],
            "completeness":{
                "status":"COMPLETE_TASK",
                "boundaries":[],
                "omitted":{}
            }
        });

        let bounded = enforce_budget(pack, 4_096).unwrap();

        assert_eq!(bounded["completeness"]["status"], "COMPLETE_TASK");
        assert_eq!(bounded["editSurfaces"].as_array().unwrap().len(), 2);
        assert_eq!(bounded["editSurfaces"][0]["name"], "main");
        assert_eq!(
            bounded["editSurfaces"][0]["sourceText"]
                .as_str()
                .unwrap()
                .len(),
            2_000
        );
        assert_eq!(bounded["editSurfaces"][1]["name"], "loadRecords");
        assert_eq!(bounded["editSurfaces"][1]["sourceText"], "");
        assert_eq!(bounded["completeness"]["omitted"]["editSurfaces"], 0);
        assert_eq!(bounded["completeness"]["omitted"]["sourceBytes"], 2_000);
    }

    #[test]
    fn stdout_budget_never_reports_trimmed_required_source_as_complete() {
        let pack = json!({
            "task":{},
            "editSurfaces":[{"name":"main","required":true,"sourceText":"r".repeat(6_000)}],
            "executionPath":[],
            "contracts":[],
            "tests":[],
            "completeness":{"status":"COMPLETE_TASK","boundaries":[],"omitted":{}}
        });

        let bounded = enforce_budget(pack, 4_096).unwrap();

        assert_eq!(bounded["completeness"]["status"], "PARTIAL_TASK");
        assert!(
            bounded["completeness"]["omitted"]["sourceBytes"]
                .as_u64()
                .unwrap()
                > 0
        );
    }

    #[test]
    fn primary_root_prefers_rich_goal_evidence_over_a_generic_exact_verb() {
        let mut update = function_candidate("update");
        update.score = 360;
        update.reasons = BTreeSet::from(["exact:update".into()]);
        update.source_text = "fun update(key: UUID)".into();
        let mut reconcile = function_candidate("reconcile");
        reconcile.score = 120;
        reconcile.reasons = BTreeSet::from(["intent-name:reconcile".into()]);
        reconcile.source_text =
            "fun reconcile(records: List<Record>) { emit(APPLIED, recordKey, payload) }".into();
        let mut selection = selection(vec![update.clone(), reconcile.clone()]);
        selection.intent_tokens = BTreeSet::from([
            "reconcile".into(),
            "applied".into(),
            "payload".into(),
            "record".into(),
            "recordkey".into(),
        ]);
        selection.goal_tokens = BTreeSet::from([
            "reconcile".into(),
            "applied".into(),
            "payload".into(),
            "record".into(),
            "recordkey".into(),
        ]);

        assert_eq!(
            selection.root_symbols(1),
            vec!["com.acme.Service.reconcile"]
        );
    }

    #[test]
    fn follows_the_call_whose_parameter_matches_task_intent() {
        let mut query_candidate = function_candidate("persistBatch");
        query_candidate.source_text = "@Query fun persistBatch() = Unit".into();
        let mut selection = selection(vec![function_candidate("emitChange"), query_candidate]);
        selection.intent_tokens = BTreeSet::from(["subjectid".into()]);
        let resolutions = vec![json!({
            "declaration":{"legacySymbolId":"com.acme.Service.run"},
            "resolvedCalls":[
                {
                    "symbol":"com.acme.Service.persistBatch",
                    "argumentToParameter":[{"parameter":"items"}]
                },
                {
                    "symbol":"com.acme.Service.emitChange",
                    "argumentToParameter":[{"parameter":"subjectId"}]
                }
            ]
        })];

        assert_eq!(
            selection.followup_symbols(&resolutions, 1),
            vec!["com.acme.Service.emitChange"]
        );
    }

    #[test]
    fn contract_rank_prefers_requested_properties_over_domain_name_overlap() {
        let mut payload = function_candidate("ChangePayload");
        payload.declaration["kind"] = json!("KtClass");
        payload.source_text = "data class ChangePayload(val key: String, val label: String)".into();
        let mut request = function_candidate("RecordRequest");
        request.declaration["kind"] = json!("KtClass");
        request.source_text = "data class RecordRequest(val filter: String)".into();
        let mut emit = function_candidate("emitChange");
        emit.source_text = "fun emitChange(payload: ChangePayload)".into();
        let mut execute = function_candidate("execute");
        execute.source_text = "fun execute(request: RecordRequest)".into();
        let calls = vec![&emit, &execute];
        let intent = BTreeSet::from(["record".into(), "key".into(), "label".into()]);

        assert!(
            contract_rank(&payload, &intent, &calls) > contract_rank(&request, &intent, &calls)
        );
    }

    #[test]
    fn anchored_test_ranking_prefers_the_primary_graph_root() {
        let root = function_candidate("executeChange");
        let helper = function_candidate("emitPayload");
        let selection = selection(Vec::new());

        let needles = task_needles(
            &[],
            "change payload",
            &[&root],
            &[&root, &helper],
            &selection,
        );

        assert_eq!(needles["executeChange"], 40);
        assert_eq!(needles["emitPayload"], 12);
    }
}
