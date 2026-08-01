use crate::canonical;
use crate::error::{ErrorCode, SthreadError};
use crate::model::ThreadIr;
use serde_json::{Map, Value, json};
use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

const MAX_EDIT_SURFACES: usize = 4;
const MAX_CONTRACTS: usize = 1;
const MAX_TESTS: usize = 1;
const MAX_EXECUTION_EDGES: usize = 4;
const MAX_SOURCE_BYTES: usize = 4_200;
const MAX_TEST_BYTES: usize = 4_200;

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
pub struct TaskContextSelection {
    files: Vec<SourceFile>,
    catalog: Vec<Candidate>,
    intent_tokens: BTreeSet<String>,
    goal_tokens: BTreeSet<String>,
    explicit_owners: BTreeSet<String>,
}

impl TaskContextSelection {
    pub fn root_symbols(&self, limit: usize) -> Vec<String> {
        let mut roots = Vec::new();
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
                roots.push(symbol);
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
        roots.extend(
            ranked
                .into_iter()
                .filter_map(|candidate| candidate.declaration["legacySymbolId"].as_str()),
        );
        let mut unique = Vec::new();
        for root in roots {
            if !unique.iter().any(|existing| existing == root) {
                unique.push(root.to_owned());
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
            } else if token.len() >= 6
                && !matches!(token.as_str(), "product" | "products" | "entity")
                && name.contains(token.as_str())
            {
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
) -> Result<TaskContextSelection, SthreadError> {
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
    Ok(TaskContextSelection {
        files,
        catalog,
        intent_tokens,
        goal_tokens,
        explicit_owners,
    })
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
    pub max_bytes: usize,
}

pub fn build(input: TaskContextBuild<'_>) -> Result<(Value, Value), SthreadError> {
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
        max_bytes,
    } = input;
    if max_bytes < 4_096 {
        return Err(SthreadError::new(
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
    let contract_declarations = collect_contracts(index_facts, selection, resolutions);
    let projection_fields = collect_projection_fields(selection, &call_declarations);
    let mut edit_candidates = root_candidates.clone();
    for candidate in &call_declarations {
        if !edit_candidates.iter().any(|existing| {
            existing.declaration["declarationId"] == candidate.declaration["declarationId"]
        }) {
            edit_candidates.push(candidate);
        }
    }
    edit_candidates.sort_by_key(|candidate| {
        Reverse(surface_rank(
            candidate,
            &root_symbols,
            &selection.intent_tokens,
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
        .map(|candidate| {
            compact_surface(
                candidate,
                body_anchors.get(
                    candidate.declaration["legacySymbolId"]
                        .as_str()
                        .unwrap_or_default(),
                ),
            )
        })
        .collect::<Vec<_>>();
    let contracts = contract_declarations
        .iter()
        .filter(|contract| {
            !edit_candidates.iter().any(|surface| {
                surface.declaration["declarationId"] == contract.declaration["declarationId"]
            })
        })
        .take(MAX_CONTRACTS)
        .map(|candidate| compact_contract(candidate))
        .collect::<Vec<_>>();
    let execution_path = collect_execution_path(resolutions, index_facts, selection);
    let test_needles = task_needles(terms, intent, &root_candidates, &edit_candidates, selection);
    let tests = collect_anchored_tests(selection, &test_needles);
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
    let thread_id = threads
        .first()
        .map(|thread| thread.thread_id.as_str())
        .unwrap_or_default();
    let full = json!({
        "schema":"semantic-task-context/0.2",
        "task":{"intent":intent,"terms":terms,"intentTokens":selection.intent_tokens,"matchedTerms":matched_terms,"unmatchedTerms":unmatched_terms},
        "snapshot":{"baseRevision":base_revision,"projectModelHash":project["projectModelHash"],"indexSnapshot":index_snapshot,"compilerVersion":project["compilerVersion"],"compilation":compilation},
        "threadId":thread_id,
        "editSurfaces":edit_surfaces,
        "executionPath":execution_path,
        "projectionFields":projection_fields,
        "contracts":contracts,
        "tests":tests,
        "validationPlan":validation_plan,
        "editPlan":{
            "schema":"semantic-task-edit-plan/0.1",
            "threadId":thread_id,
            "baseRevision":base_revision,
            "operationShape":{"kind":"REWRITE_DECLARATION","target":{"targetId":"<emitted targetId>"},"preconditions":{"substitutions":[{"old":"...","new":"..."}]}},
            "instruction":"Use kind and target.targetId. Prefer REWRITE_DECLARATION substitutions. Multiline: oldLines/newLines/kotlinLines (common-dedented); occurrence selects one match, occurrences edits all. A typed contract must not become Any/Any?: required fields stay statically accessible and existing payloads assignable. CREATE_FILE adds top-level declarations."
        },
        "evidence":evidence_display(repo,evidence_path),
        "completeness":{
            "status":if boundaries.is_empty(){"COMPLETE_TASK"}else{"PARTIAL_TASK"},
            "boundaries":boundaries,
            "coverage":{"roots":root_candidates.len(),"resolvedCalls":execution_path.len(),"internalCalls":internal_calls,"contracts":contracts.len(),"tests":tests.len()},
            "stdoutLimitBytes":max_bytes,
            "omitted":{"editSurfaces":0,"executionPath":0,"contracts":0,"tests":0,"sourceBytes":0}
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
    let (source_text, truncated, omitted) = truncate_utf8(&candidate.source_text, MAX_SOURCE_BYTES);
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
        ("sourceText".into(), json!(source_text)),
        ("declarationTarget".into(), declaration_target),
    ]);
    if let Some(anchor) = body_anchor {
        let mut compact = anchor.clone();
        if let Some(object) = compact.as_object_mut() {
            object.remove("sourceText");
        }
        value.insert("bodyTarget".into(), compact);
    }
    if truncated {
        value.insert("sourceTruncated".into(), json!(true));
        value.insert("sourceBytesOmitted".into(), json!(omitted));
    }
    Value::Object(value)
}

fn compact_contract(candidate: &Candidate) -> Value {
    let (source_text, truncated, omitted) = truncate_utf8(&candidate.source_text, MAX_SOURCE_BYTES);
    json!({
        "name":candidate.declaration["name"],
        "kind":candidate.declaration["kind"],
        "file":candidate.declaration["file"],
        "lines":[candidate.line_start,candidate.line_end],
        "sourceText":source_text,
        "sourceTruncated":truncated,
        "sourceBytesOmitted":omitted,
        "declarationTarget":declaration_target(candidate)
    })
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
    let name = candidate.declaration["name"]
        .as_str()
        .unwrap_or_default()
        .to_lowercase();
    candidate.score
        + usize::from(root_symbols.contains(symbol)) * 2_000
        + call_declaration_rank(candidate, intent_tokens)
        + usize::from(
            ["event", "entity", "dto", "projection"]
                .iter()
                .any(|suffix| name.contains(suffix)),
        ) * 400
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
    contracts.sort_by_key(|candidate| Reverse(contract_rank(candidate, &selection.intent_tokens)));
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

fn contract_rank(candidate: &Candidate, intent_tokens: &BTreeSet<String>) -> usize {
    let name = candidate.declaration["name"]
        .as_str()
        .unwrap_or_default()
        .to_lowercase();
    let preferred = ["entity", "event", "dto", "projection", "contract"]
        .iter()
        .filter(|fragment| name.contains(**fragment))
        .count();
    let incidental = ["filter", "request", "action", "type", "context", "metadata"]
        .iter()
        .filter(|fragment| name.contains(**fragment))
        .count();
    let intent_hits = intent_tokens
        .iter()
        .filter(|token| token.len() >= 4 && name.contains(token.as_str()))
        .count();
    (candidate.score + preferred * 500 + intent_hits * 100usize)
        .saturating_sub(incidental.min(2) * 200usize)
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
            let root_target = resolutions.iter().any(|candidate| {
                candidate["declaration"]["declarationId"] == target["declarationId"]
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
    needles
}

fn collect_anchored_tests(
    selection: &TaskContextSelection,
    needles: &BTreeMap<String, usize>,
) -> Vec<Value> {
    let mut candidates = selection
        .files
        .iter()
        .filter(|file| file.is_test)
        .filter_map(|file| test_snippet(file, needles))
        .collect::<Vec<_>>();
    candidates.sort_by_key(|candidate| Reverse(candidate["score"].as_u64().unwrap_or_default()));
    candidates.truncate(MAX_TESTS);
    candidates
}

fn test_snippet(file: &SourceFile, needles: &BTreeMap<String, usize>) -> Option<Value> {
    let lines = file.source.lines().collect::<Vec<_>>();
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
            let matched = needles
                .iter()
                .filter(|(needle, _)| contains_name(&source, needle))
                .map(|(needle, _)| needle.clone())
                .collect::<Vec<_>>();
            let score = matched
                .iter()
                .filter_map(|needle| needles.get(needle))
                .sum::<usize>();
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
    let (source_text, truncated, omitted) = truncate_utf8(declaration_source, MAX_TEST_BYTES);
    let declaration_name = test_function_name(lines[function_line]).unwrap_or("test");
    let exact_text_hash = canonical::hash_bytes(declaration_source.as_bytes());
    let anchor_id = format!(
        "test-declaration:{}",
        canonical::hash_bytes(
            format!("{}:{declaration_name}:{exact_text_hash}", file.path).as_bytes()
        )
    );
    Some(json!({
        "path":file.path,
        "lines":[start+1,end+1],
        "matched":matched,
        "score":score,
        "sourceText":source_text,
        "sourceTruncated":truncated,
        "sourceBytesOmitted":omitted,
        "declarationTarget":{
            "anchorId":anchor_id,
            "declarationId":anchor_id,
            "fileId":file.path,
            "ownerSymbolId":declaration_name,
            "syntaxKind":"KtNamedFunction",
            "exactTextHash":exact_text_hash,
            "rangeHint":[start+1,end+1]
        }
    }))
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

fn validation_plan(project: &Value, tests: &[Value]) -> Value {
    let build_system = project["buildSystem"].as_str().unwrap_or("GRADLE");
    let launcher = project["buildLauncher"]
        .as_str()
        .unwrap_or(match build_system {
            "MAVEN" => "mvn",
            _ => "./gradlew",
        });
    let stems = tests
        .iter()
        .filter_map(|test| test["path"].as_str())
        .filter_map(|path| Path::new(path).file_stem()?.to_str())
        .collect::<BTreeSet<_>>();
    let targeted_args = match build_system {
        "MAVEN" if !stems.is_empty() => vec![
            format!("-Dtest={}", stems.into_iter().collect::<Vec<_>>().join(",")),
            "test".into(),
        ],
        "GRADLE" if !stems.is_empty() => {
            let mut args = vec!["cleanTest".into()];
            for stem in stems {
                args.push("--tests".into());
                args.push(format!("*{stem}"));
            }
            args
        }
        _ => project["testTasks"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect(),
    };
    json!({
        "buildSystem":build_system,
        "buildLauncher":launcher,
        "compileTask":project["compileTask"],
        "targetedArgs":targeted_args,
        "cleanDetachedWorktree":true
    })
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
        if !left.is_alphanumeric() {
            starts.push(right_index);
        } else if left.is_lowercase() && right.is_uppercase() {
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
    for key in ["editSurfaces", "contracts", "tests"] {
        for item in context
            .get_mut(key)
            .and_then(Value::as_array_mut)
            .into_iter()
            .flatten()
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
                let id = target["anchorId"].clone();
                item[id_key] = id;
                item.as_object_mut()
                    .expect("context item")
                    .remove(target_key);
            }
        }
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

fn scan_kotlin_sources(repo: &Path) -> Result<Vec<SourceFile>, SthreadError> {
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
                .map_err(|error| SthreadError::new(ErrorCode::Internal, error.to_string()))?
                .to_string_lossy()
                .replace('\\', "/");
            let source = std::fs::read_to_string(path)
                .map_err(|error| SthreadError::new(ErrorCode::InvalidInput, error.to_string()))?;
            let is_test = relative.starts_with("src/test/") || relative.contains("/src/test/");
            Ok(SourceFile {
                path: relative,
                source,
                is_test,
            })
        })
        .collect::<Result<Vec<_>, SthreadError>>()?;
    files.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(files)
}

fn enforce_budget(mut pack: Value, max_bytes: usize) -> Result<Value, SthreadError> {
    let original = ["editSurfaces", "executionPath", "contracts", "tests"]
        .into_iter()
        .map(|key| (key.to_owned(), array_len(&pack, key)))
        .collect::<BTreeMap<_, _>>();
    let mut source_bytes = 0usize;
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
        if let Some(omitted) = trim_largest_source(&mut pack) {
            source_bytes += omitted;
            continue;
        }
        if array_len(&pack, "editSurfaces") > 1 && pop_array(&mut pack, "editSurfaces") {
            continue;
        }
        return Err(SthreadError::new(
            ErrorCode::InvalidInput,
            format!("--max-bytes {max_bytes} is too small for the mandatory task closure"),
        ));
    }
    let omitted = original
        .iter()
        .map(|(key, count)| (key.clone(), count - array_len(&pack, key)))
        .collect::<BTreeMap<_, _>>();
    let partial = source_bytes > 0 || omitted.values().any(|count| *count > 0);
    if partial {
        pack["completeness"]["status"] = json!("PARTIAL_BUDGET");
        pack["completeness"]["boundaries"]
            .as_array_mut()
            .expect("boundaries array")
            .push(json!({"kind":"STDOUT_BUDGET","omitted":omitted,"sourceBytes":source_bytes}));
    }
    pack["completeness"]["omitted"] = json!({
        "editSurfaces":omitted.get("editSurfaces").copied().unwrap_or_default(),
        "executionPath":omitted.get("executionPath").copied().unwrap_or_default(),
        "contracts":omitted.get("contracts").copied().unwrap_or_default(),
        "tests":omitted.get("tests").copied().unwrap_or_default(),
        "sourceBytes":source_bytes
    });
    if serialized_len(&pack)? > max_bytes {
        return Err(SthreadError::new(
            ErrorCode::Internal,
            "task context budget accounting failed",
        ));
    }
    Ok(pack)
}

fn trim_largest_source(pack: &mut Value) -> Option<usize> {
    let mut best: Option<(String, usize, usize)> = None;
    for key in ["editSurfaces", "contracts", "tests"] {
        for (index, item) in pack.get(key)?.as_array()?.iter().enumerate() {
            let length = item["sourceText"]
                .as_str()
                .map(str::len)
                .unwrap_or_default();
            if length > 640
                && best
                    .as_ref()
                    .is_none_or(|(_, _, best_length)| length > *best_length)
            {
                best = Some((key.to_owned(), index, length));
            }
        }
    }
    let (key, index, length) = best?;
    let item = pack.get_mut(&key)?.as_array_mut()?.get_mut(index)?;
    let source = item["sourceText"].as_str()?.to_owned();
    let (truncated, _, omitted) = truncate_utf8(&source, (length / 2).max(640));
    item["sourceText"] = json!(truncated);
    item["sourceTruncated"] = json!(true);
    let previous = item["sourceBytesOmitted"].as_u64().unwrap_or_default() as usize;
    item["sourceBytesOmitted"] = json!(previous + omitted);
    Some(omitted)
}

fn serialized_len(value: &Value) -> Result<usize, SthreadError> {
    canonical::pretty(value)
        .map(|text| text.len() + 1)
        .map_err(|error| SthreadError::new(ErrorCode::Internal, error.to_string()))
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

    #[test]
    fn intent_tokens_split_camel_case_and_keep_payload_fields() {
        let tokens = task_tokens(
            "archive typed id/code/title entity",
            &["ProductService".into()],
        );
        assert!(tokens.contains("archive"));
        assert!(tokens.contains("id"));
        assert!(tokens.contains("typed"));
    }

    #[test]
    fn plausible_terms_are_split_and_inflected_intent_is_normalized() {
        let tokens = task_tokens(
            "When archiving products",
            &["archiveProduct".into(), "ProductChangeFeed".into()],
        );
        assert!(tokens.contains("archive"));
        assert!(tokens.contains("product"));
        assert!(tokens.contains("feed"));
    }

    #[test]
    fn enum_literal_is_not_treated_as_an_exact_declaration_name() {
        assert!(looks_like_constant("DELETED"));
        assert!(looks_like_constant("PRODUCT_DELETED"));
        assert!(!looks_like_constant("archiveProduct"));
    }

    #[test]
    fn primary_root_prefers_rich_goal_evidence_over_a_generic_exact_verb() {
        let mut delete = function_candidate("delete");
        delete.score = 360;
        delete.reasons = BTreeSet::from(["exact:delete".into()]);
        delete.source_text = "fun delete(id: UUID)".into();
        let mut archive = function_candidate("archive");
        archive.score = 120;
        archive.reasons = BTreeSet::from(["intent-name:archive".into()]);
        archive.source_text =
            "fun archive(products: List<Product>) { emit(DELETED, productId, entity) }".into();
        let selection = TaskContextSelection {
            files: Vec::new(),
            catalog: vec![delete.clone(), archive.clone()],
            intent_tokens: BTreeSet::from([
                "archive".into(),
                "deleted".into(),
                "entity".into(),
                "product".into(),
                "productid".into(),
            ]),
            goal_tokens: BTreeSet::from([
                "archive".into(),
                "deleted".into(),
                "entity".into(),
                "product".into(),
                "productid".into(),
            ]),
            explicit_owners: BTreeSet::new(),
        };

        assert_eq!(selection.root_symbols(1), vec!["com.acme.Service.archive"]);
    }

    #[test]
    fn follows_the_call_whose_parameter_matches_task_intent() {
        let mut query_candidate = function_candidate("persistBatch");
        query_candidate.source_text = "@Query fun persistBatch() = Unit".into();
        let selection = TaskContextSelection {
            files: Vec::new(),
            catalog: vec![function_candidate("emitChange"), query_candidate],
            intent_tokens: BTreeSet::from(["subjectid".into()]),
            goal_tokens: BTreeSet::new(),
            explicit_owners: BTreeSet::new(),
        };
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
    fn anchored_test_ranking_prefers_the_primary_graph_root() {
        let root = function_candidate("executeChange");
        let helper = function_candidate("emitPayload");
        let selection = TaskContextSelection {
            files: Vec::new(),
            catalog: Vec::new(),
            intent_tokens: BTreeSet::new(),
            goal_tokens: BTreeSet::new(),
            explicit_owners: BTreeSet::new(),
        };

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
