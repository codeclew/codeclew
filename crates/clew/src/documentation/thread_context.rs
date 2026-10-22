//! Resumable explanation accounting over retained Work, separate from read receipts.
//! Lexical candidates are navigation obligations, never resolved execution edges.
use super::{
    digest, invalid,
    model::*,
    process_candidates, process_graph,
    store::{self, Repository},
    work::{self, Work},
};
use crate::error::ClewError;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

pub(super) const POLICY: &str = "retained-thread-preparation/1";

fn scope(o: &Observation) -> &str {
    o.normalized["scope"].as_str().unwrap_or("")
}
fn row(kind: &str, key: &str, record: Value) -> Value {
    json!({"kind":"THREAD_OBLIGATION","id":format!("{kind}:{key}"),"record":record,"obligationKind":kind})
}
fn locations(e: &ServiceEvidence, ids: &[String]) -> Vec<Value> {
    ids.iter().filter_map(|id|e.sources.get(id)).map(|s|json!({"sourceId":s.id,"file":s.file,"startLine":s.start_line,"endLine":s.end_line,"textDigest":s.text_digest,"revision":s.revision})).collect()
}
fn declaration(e: &ServiceEvidence, id: &str) -> Result<String, ClewError> {
    let id = super::declaration_migration::canonical(e, id)?;
    if !e
        .observations
        .get(&id)
        .is_some_and(process_graph::callable_observation)
    {
        return Err(invalid(
            "THREAD_ROOT_REQUIRED: use an exact retained callable declaration ID",
        ));
    }
    Ok(id)
}
fn evidence(w: &Work) -> Result<&ServiceEvidence, ClewError> {
    let service = w
        .subject
        .strip_prefix("service:")
        .ok_or_else(|| invalid("thread preparation requires service Work"))?;
    w.checked
        .services
        .get(service)
        .ok_or_else(|| invalid("thread service evidence unavailable"))
}

fn entry_roots(e: &ServiceEvidence, entry: &Entrypoint) -> Vec<String> {
    let scopes: BTreeSet<_> = entry
        .dependency_ids
        .iter()
        .filter_map(|id| e.observations.get(id))
        .filter_map(|o| o.normalized["scope"].as_str())
        .collect();
    e.observations
        .values()
        .filter(|o| {
            process_graph::callable_observation(o)
                && o.symbol == entry.symbol
                && if scopes.is_empty() {
                    scope(o).is_empty()
                } else {
                    scopes.contains(scope(o))
                }
        })
        .map(|o| o.id.clone())
        .collect()
}
pub(super) fn inventory(e: &ServiceEvidence) -> Result<Vec<Value>, ClewError> {
    let mut rows = Vec::new();
    for o in e
        .observations
        .values()
        .filter(|o| matches!(o.kind.as_str(), "SOURCE_SCOPE" | "CAPTURE_SCOPE"))
    {
        let mut summary = o.normalized.clone();
        summary.as_object_mut().map(|o| o.remove("inventory"));
        summary.as_object_mut().map(|o| o.remove("excludedFiles"));
        rows.push(json!({"kind":"THREAD_SCOPE","id":o.id,"record":{"summary":summary,"dependencyIds":[o.id]}}));
        for (path, file) in o.normalized["inventory"].as_object().into_iter().flatten() {
            rows.push(json!({"kind":"THREAD_FILE","id":format!("{}:{path}",o.id),"record":{"path":path,"input":file,"dependencyIds":[o.id]}}));
        }
        for path in o.normalized["excludedFiles"]
            .as_array()
            .into_iter()
            .flatten()
        {
            rows.push(json!({"kind":"THREAD_EXCLUDED_FILE","id":format!("{}:excluded:{}",o.id,path),"record":{"path":path,"reason":"OUTSIDE_SELECTED_COMPILATION","dependencyIds":[o.id]}}));
        }
    }
    for entry in &e.entrypoints {
        if !process_candidates::public_boundary(entry) && entry.kind != "SOURCE_HTTP_MAPPING" {
            continue;
        }
        let roots = entry_roots(e, entry);
        let source_ids: BTreeSet<_> = entry
            .source_ids
            .iter()
            .cloned()
            .chain(
                roots
                    .iter()
                    .flat_map(|id| e.observations[id].source_ids.iter().cloned()),
            )
            .collect();
        rows.push(json!({"kind":"THREAD_ENTRY","id":entry.id,"record":{"entrypoint":entry,"rootDeclarations":roots,
            "classification":if process_candidates::public_boundary(entry){"DISCOVERED_TRIGGER"}else{"POSSIBLE_SOURCE_SURFACE"},
            "sourceIds":source_ids,"dependencyIds":entry.dependency_ids,"rootResolution":if roots.len()==1{"UNIQUE_RETAINED_TARGET"}else{"MISSING_OR_AMBIGUOUS_TARGET"},"explanationStatus":"NOT_STARTED",
            "nextAction":{"query":{"kind":"THREAD_CONTEXT","symbolContains":roots.first()}},"runtimeRegistration":"NOT_ESTABLISHED"}}));
    }
    let represented_sources: BTreeSet<_> = rows
        .iter()
        .filter(|r| r["kind"] == "THREAD_ENTRY")
        .flat_map(|r| {
            r["record"]["sourceIds"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .map(str::to_owned)
        })
        .collect();
    for o in e
        .observations
        .values()
        .filter(|o| o.kind == "ENTRY_SURFACE")
    {
        // Source audits may corroborate an existing compiler/syntax entry. Keep
        // unsupported surfaces visible, without doubling its discovered roots.
        if represented_sources
            .iter()
            .filter_map(|id| e.sources.get(id))
            .any(|a| {
                o.source_ids
                    .iter()
                    .filter_map(|id| e.sources.get(id))
                    .any(|b| {
                        a.file == b.file && a.start_line <= b.end_line && b.start_line <= a.end_line
                    })
            })
        {
            continue;
        }
        let roots: Vec<_> = e
            .observations
            .values()
            .filter(|d| {
                process_graph::callable_observation(d)
                    && (scope(d).is_empty()
                        || o.normalized["scope"].as_str().is_none_or(|s| s == scope(d)))
                    && d.source_ids.iter().any(|id| {
                        e.sources.get(id).is_some_and(|a| {
                            o.source_ids.iter().any(|id| {
                                e.sources.get(id).is_some_and(|b| {
                                    a.file == b.file
                                        && a.start_line <= b.end_line
                                        && b.start_line <= a.end_line
                                })
                            })
                        })
                    })
            })
            .map(|d| d.id.clone())
            .collect();
        rows.push(json!({"kind":"THREAD_ENTRY","id":o.id,"record":{"classification":"POSSIBLE_SOURCE_SURFACE","surface":o.normalized,"rootDeclarations":roots,
            "sourceIds":o.source_ids,"dependencyIds":[o.id],"explanationStatus":"NOT_STARTED","runtimeRegistration":"NOT_ESTABLISHED"}}));
    }
    rows.sort_by_key(|r| r["id"].as_str().unwrap_or_default().to_owned());
    Ok(rows)
}

/// No source recapture or provider execution. One row per obligation allows the
/// existing Work pager to resume large candidate lists without dropping names.
pub(super) fn context(
    e: &ServiceEvidence,
    id: &str,
    snapshot: &str,
    input: &str,
    budget: &super::process_extract::Budgets,
) -> Result<Vec<Value>, ClewError> {
    let id = declaration(e, id)?;
    let root = &e.observations[&id];
    let syntax = scope(root).is_empty() && root.normalized["authority"] == "SYNTAX";
    if !syntax && scope(root).is_empty() {
        return Err(invalid(
            "thread root has neither retained compiler scope nor source-syntax authority",
        ));
    }
    let mut rows = Vec::new();
    let mut queue = VecDeque::from([id.clone()]);
    let mut seen = BTreeSet::new();
    let mut name_index: BTreeMap<&str, Vec<&Observation>> = BTreeMap::new();
    for o in e
        .observations
        .values()
        .filter(|o| process_graph::callable_observation(o) && scope(o) == scope(root))
    {
        if let Some(name) = o.normalized["name"].as_str() {
            name_index.entry(name).or_default().push(o);
        }
    }
    let graph = if syntax {
        Value::Null
    } else {
        process_graph::collect_bounded(e, root, snapshot, input, Some(budget))?
    };
    if !syntax {
        queue = graph["methods"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|m| m["id"].as_str().map(str::to_owned))
            .collect();
        for frontier in graph["frontiers"].as_array().into_iter().flatten() {
            if frontier["kind"] == "SERVICE_PROVIDER_BOUNDARY" {
                let code = frontier["detail"].as_str().unwrap_or("");
                let local_flow_summary = matches!(
                    code,
                    "EXCEPTION_FLOW_REQUIRES_SOURCE_REVIEW"
                        | "SHORT_CIRCUIT_FLOW_REQUIRES_SOURCE_REVIEW"
                        | "LAMBDA_EXECUTION_NOT_EXPANDED"
                ) && e.observations.values().any(|o| {
                    process_graph::callable_observation(o)
                        && scope(o) == scope(root)
                        && o.normalized["documentation"]["boundaries"]
                            .as_array()
                            .is_some_and(|b| b.iter().any(|b| b == code))
                });
                let local_variable_summary = code == "JAVA_VARIABLE_LAMBDA_DEFERRED"
                    && e.observations.values().any(|o| {
                        o.kind == "PROVIDER_BOUNDARY"
                            && scope(o) == scope(root)
                            && o.normalized["code"] == code
                    });
                if local_flow_summary || local_variable_summary {
                    // Relevant method/flow or anchored provider obligations below
                    // retain this limitation. The service summary has no owner.
                    continue;
                }
            }
            let key = digest(frontier)?;
            let anchor = frontier["eventId"]
                .as_str()
                .or_else(|| frontier["methodId"].as_str())
                .or_else(|| frontier["fromMethodId"].as_str());
            let sources = anchor
                .and_then(|id| e.observations.get(id))
                .map(|o| o.source_ids.clone())
                .unwrap_or_else(|| root.source_ids.clone());
            let external = matches!(
                frontier["kind"].as_str(),
                Some(
                    "EXTERNAL_OR_UNRETAINED_TARGET"
                        | "CALL_BINDING_PROVIDER_UNSUPPORTED"
                        | "METHOD_PROVIDER_BOUNDARY"
                        | "PROVIDER_FLOW_BOUNDARY"
                        | "PROVIDER_FLOW_SLOTS_UNAVAILABLE"
                )
            );
            rows.push(row("BOUNDARY",&key,json!({"frontier":frontier,"sourceIds":sources,"dependencyIds":anchor.into_iter().collect::<Vec<_>>(),"actionable":!external,"nextAction":if external{"explain the external or unretained target boundary; runtime dispatch and effects remain unknown"}else{"inspect retained graph frontier; expand budget or capture missing evidence explicitly"}})));
        }
    }
    let mut flow_index: BTreeMap<(&str, &str), Vec<&Observation>> = BTreeMap::new();
    for o in e.observations.values().filter(|o| o.kind == "FLOW") {
        flow_index.entry((scope(o), &o.symbol)).or_default().push(o);
    }
    let graph_events: BTreeSet<_> = graph["events"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|event| event["id"].as_str())
        .collect();
    let mut effects_index: BTreeMap<(&str, &str), Vec<&Observation>> = BTreeMap::new();
    for o in e.observations.values().filter(|o| {
        o.kind == "VARIABLE_ACCESS"
            && matches!(
                o.normalized["accessMode"].as_str(),
                Some("WRITE" | "READ_WRITE")
            )
    }) {
        effects_index
            .entry((scope(o), &o.symbol))
            .or_default()
            .push(o);
    }
    let mut candidate_rows = 0usize;
    let mut event_count = 0;
    let mut source_bytes = 0;
    while let Some(id) = queue.pop_front() {
        if seen.contains(&id) {
            continue;
        }
        let method = &e.observations[&id];
        let method_events: Vec<_> = flow_index
            .get(&(scope(method), method.symbol.as_str()))
            .into_iter()
            .flatten()
            .copied()
            .filter(|event| event.belongs_to_declaration(method))
            .filter(|event| syntax || graph_events.contains(event.id.as_str()))
            .collect();
        let effects = effects_index
            .get(&(scope(method), method.symbol.as_str()))
            .map(Vec::as_slice)
            .unwrap_or_default();
        let bytes: usize = method
            .source_ids
            .iter()
            .filter_map(|id| e.sources.get(id))
            .map(|s| s.text.len())
            .sum();
        if seen.len() >= budget.max_methods
            || event_count + method_events.len() + effects.len() > budget.max_events
            || source_bytes + bytes > budget.max_source_bytes
        {
            rows.push(row("BUDGET",&id,json!({"declaration":id,"actionable":true,"nextAction":"read this declaration as a narrower THREAD_CONTEXT root; this parent budget cut remains open"})));
            continue;
        }
        seen.insert(id.clone());
        event_count += method_events.len() + effects.len();
        source_bytes += bytes;
        let source_available = !method.source_ids.is_empty()
            && method
                .source_ids
                .iter()
                .all(|id| e.sources.contains_key(id));
        rows.push(row("BODY",&id,json!({"declaration":id,"symbol":method.symbol,"sourceIds":method.source_ids,"dependencyIds":[id],
            "actionable":!source_available,"nextAction":"read complete retained declaration and source through these Work references; account for inputs, effects and outcomes",
            "bodyStatus":method.normalized["documentation"]["bodyStatus"],"authority":if syntax{"SOURCE_BODY_NOT_CALL_RESOLUTION"}else{"RETAINED_COMPILER_DECLARATION_NOT_RUNTIME_DISPATCH"}})));
        for boundary in e.observations.values().filter(|o| {
            o.kind == "PROVIDER_BOUNDARY"
                && scope(o) == scope(method)
                && o.source_ids
                    .iter()
                    .filter_map(|id| e.sources.get(id))
                    .any(|b| {
                        method
                            .source_ids
                            .iter()
                            .filter_map(|id| e.sources.get(id))
                            .any(|m| {
                                b.file == m.file
                                    && m.start_line <= b.start_line
                                    && b.end_line <= m.end_line
                            })
                    })
        }) {
            let reviewable = boundary.normalized["code"] == "JAVA_VARIABLE_LAMBDA_DEFERRED";
            rows.push(row("BOUNDARY", &boundary.id, json!({"declaration":id,
                "frontier":{"kind":"SOURCE_PROVIDER_BOUNDARY","detail":boundary.normalized},
                "sourceIds":boundary.source_ids,"dependencyIds":[boundary.id],"actionable":!reviewable,
                "nextAction":"explain the source-bound provider limitation; deferred execution and runtime effects remain unknown"})));
        }
        for access in effects {
            rows.push(row("DATA_EFFECT",&access.id,json!({"declaration":id,"effect":access.normalized,"sourceIds":access.source_ids,"dependencyIds":[access.id],"actionable":false,
                "nextAction":"explain this source-bound write under its surrounding conditions; it does not establish durable storage, delivery or runtime execution"})));
        }
        for event in method_events {
            rows.push(row("EVENT",&event.id,json!({"declaration":id,"event":event.normalized,"sourceIds":event.source_ids,"dependencyIds":[event.id],"actionable":false,
                "nextAction":"explain this event in its parent/control context; deferred callbacks and failures are separate obligations"})));
            if !matches!(
                event.normalized["kind"].as_str(),
                Some("CALL" | "CONSTRUCT")
            ) {
                continue;
            }
            if syntax {
                let hint = event.normalized["navigationName"].as_str();
                let candidates = hint
                    .and_then(|name| name_index.get(name))
                    .map(Vec::as_slice)
                    .unwrap_or_default();
                if candidates.is_empty() {
                    rows.push(row("CALL_BOUNDARY",&event.id,json!({"declaration":id,"eventId":event.id,"sourceIds":event.source_ids,"dependencyIds":[event.id],
                        "candidateCount":0,"targetStatus":event.normalized["targetStatus"],"actionable":hint.is_none(),"runtimeTarget":"UNKNOWN","reason":if hint.is_none(){"LEXICAL_CALL_NAME_NOT_CAPTURED"}else{"NO_NAME_CANDIDATE_IN_SELECTED_INPUTS"},
                        "nextAction":"explain the missing local candidate or external/unsupported receiver boundary; no transport or dispatch guarantee is established"})));
                }
                let available = budget.max_events.saturating_sub(candidate_rows);
                if candidates.len() > available {
                    rows.push(row("CANDIDATE_BUDGET",&event.id,json!({"declaration":id,"eventId":event.id,"sourceIds":event.source_ids,"dependencyIds":[event.id],"pendingCandidateCount":candidates.len()-available,"actionable":true,
                        "nextAction":{"query":{"kind":"SYMBOL","symbolContains":hint.unwrap_or("")}},"limitation":"Inspect remaining callable declarations through bounded Work navigation; omitted candidates are not certified irrelevant."})));
                }
                for target in candidates.iter().take(available) {
                    candidate_rows += 1;
                    rows.push(row("CANDIDATE",&format!("{}:{}",event.id,target.id),json!({"declaration":id,"eventId":event.id,"candidateDeclaration":target.id,
                        "candidateCount":candidates.len(),"reason":"DECLARED_NAME_MATCH_IN_SELECTED_SOURCE_PARTITION","targetStatus":event.normalized["targetStatus"],
                        "relationshipAuthority":"NAVIGATION_ONLY","runtimeTarget":"UNKNOWN","sourceLocations":locations(e,&target.source_ids),"ambiguity":if candidates.len()>1{"MULTIPLE_NAME_CANDIDATES"}else{"UNIQUE_NAME_NOT_BINDING"},"sourceIds":target.source_ids,"dependencyIds":[event.id,target.id],
                        "nextAction":"read candidate body; explain applicability or retain alternatives; a unique lexical candidate is still not compiler resolution","actionable":false})));
                    if !seen.contains(&target.id) {
                        queue.push_back(target.id.clone());
                    }
                }
            }
        }
    }
    // Compiler-selected edges are delivered as evidence, never converted into
    // lexical resolution. Interface declaration targets retain dispatch limits.
    for edge in graph["edges"].as_array().into_iter().flatten() {
        rows.push(row("CALL",&digest(edge)?,json!({"edge":edge,"sourceIds":edge["sourceIds"],"dependencyIds":edge["dependencyIds"],"actionable":false,
            "nextAction":"explain the retained call and its receiver/dispatch boundary; declaration selection does not prove runtime implementation"})));
    }
    rows.sort_by_key(|r| r["id"].as_str().unwrap_or_default().to_owned());
    Ok(rows)
}

pub(super) fn query(w: &Work, kind: &str, root: &str) -> Result<Vec<Value>, ClewError> {
    let e = evidence(w)?;
    match kind {
        "THREAD_INVENTORY" if root.is_empty() => inventory(e),
        "THREAD_CONTEXT" => context(
            e,
            root,
            w.snapshot.as_deref().unwrap_or(""),
            &w.checked.input_digest,
            &Default::default(),
        ),
        _ => Err(invalid(
            "THREAD_INVENTORY takes no filter; THREAD_CONTEXT needs an exact declaration ID",
        )),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct Account {
    pub obligation: String,
    pub explanation: String,
    /// Full retained evidence references, not navigation summaries.
    pub references: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct Assessment {
    pub schema: String,
    pub root_declaration: String,
    pub accounts: Vec<Account>,
}
#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Progress {
    work: String,
    accounts: BTreeMap<String, BTreeMap<String, Account>>,
}
fn progress_path(w: &Work) -> Result<String, ClewError> {
    Ok(format!(
        "{}/thread-progress-{}.json",
        work::directory(&w.id)?,
        &digest(&(
            POLICY,
            include_str!("thread_context.rs"),
            include_str!("thread_inventory.rs"),
            include_str!("process_graph.rs")
        ))?[7..]
    ))
}
fn load_progress(repo: &Repository, w: &Work) -> Result<Progress, ClewError> {
    let path = repo.path(&progress_path(w)?)?;
    let p: Progress = if path.exists() {
        store::read(&path, 4 * 1024 * 1024)?
    } else {
        Progress {
            work: w.id.clone(),
            ..Default::default()
        }
    };
    if p.work != w.id {
        return Err(invalid("thread progress belongs to another Work"));
    }
    Ok(p)
}
pub(super) fn accounts_status(
    rows: &[Value],
    accounts: Option<&BTreeMap<String, Account>>,
) -> Value {
    let open = rows
        .iter()
        .filter(|r| {
            r["record"]["actionable"] == true
                || !accounts.is_some_and(|a| a.contains_key(r["id"].as_str().unwrap()))
        })
        .count();
    json!({"status":if open==rows.len(){"NOT_STARTED"}else if open>0{"IN_PROGRESS"}else{"ACCOUNTED_WITH_BOUNDARIES"},"obligations":rows.len(),"open":open,"meaningReview":"UNASSESSED","runtimeClosure":"NOT_ESTABLISHED"})
}
pub(super) fn assess(repo: &Repository, id: &str, a: Assessment) -> Result<Value, ClewError> {
    let w = work::load(repo, id)?;
    if a.schema != "codeclew-thread-assessment/1.0" || a.accounts.len() > 4096 {
        return Err(invalid("invalid thread assessment"));
    }
    let e = evidence(&w)?;
    let root = declaration(e, &a.root_declaration)?;
    let rows = query(&w, "THREAD_CONTEXT", &root)?;
    let obligations: BTreeMap<_, _> = rows
        .iter()
        .map(|r| (r["id"].as_str().unwrap(), r))
        .collect();
    let state = work::read_state(repo, id)?;
    let supplied: BTreeSet<_> = state
        .receipts
        .values()
        .flat_map(|r| r.supplied.iter().map(String::as_str))
        .collect();
    let completed = super::work_parts::completed_source_references(&w, &state)?;
    for account in &a.accounts {
        let row = obligations
            .get(account.obligation.as_str())
            .ok_or_else(|| invalid("assessment obligation is outside the retained thread"))?;
        if account.explanation.trim().len() < 16
            || account.explanation.len() > 4096
            || account.references.is_empty()
            || account.references.len() > 16
        {
            return Err(invalid(
                "each thread account needs a bounded explanation and retained evidence references",
            ));
        }
        let expected: BTreeSet<_> = row["record"]["sourceIds"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .collect();
        if !account.references.iter().any(|reference| {
            w.handles
                .get(reference)
                .is_some_and(|h| h.kind == "SOURCE" && expected.contains(h.id.as_str()))
                && (supplied.contains(reference.as_str()) || completed.contains(reference))
        }) {
            return Err(invalid(
                "thread account requires a complete recorded source read for its own obligation; reading inventory or a controller does not explain downstream work",
            ));
        }
    }
    let _lock = repo.lock()?;
    let mut p = load_progress(repo, &w)?;
    let accounts = p.accounts.entry(root).or_default();
    for account in a.accounts {
        accounts.insert(account.obligation.clone(), account);
    }
    let status = accounts_status(&rows, Some(accounts));
    let encoded = super::bytes(&p)?;
    if encoded.len() > 4 * 1024 * 1024 {
        return Err(invalid("thread progress byte budget exceeded"));
    }
    repo.atomic(&progress_path(&w)?, &encoded)?;
    Ok(json!({"work":id,"policy":POLICY,"progress":status,"publication":"NOT_CHANGED"}))
}
pub(super) fn status(
    repo: &Repository,
    id: &str,
    cursor: Option<&str>,
    limit: usize,
) -> Result<Value, ClewError> {
    let w = work::load(repo, id)?;
    let e = evidence(&w)?;
    let p = load_progress(repo, &w)?;
    let inventory = inventory(e)?;
    let roots: BTreeSet<_> = inventory
        .iter()
        .flat_map(|r| {
            r["record"]["rootDeclarations"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
        })
        .collect();
    let mut entries = Vec::new();
    for root in roots {
        let progress = if let Some(accounts) = p.accounts.get(root) {
            accounts_status(&query(&w, "THREAD_CONTEXT", root)?, Some(accounts))
        } else {
            json!({"status":"NOT_STARTED","meaningReview":"UNASSESSED"})
        };
        entries.push(json!({"id":root,"rootDeclaration":root,"progress":progress}));
    }
    let scopes: Vec<_> = e
        .observations
        .values()
        .filter(|o| matches!(o.kind.as_str(), "SOURCE_SCOPE" | "CAPTURE_SCOPE"))
        .collect();
    let scope_known = !scopes.is_empty()
        && scopes.iter().all(|s| {
            s.normalized["inventory"]
                .as_object()
                .is_some_and(|i| !i.is_empty())
        });
    let inventory_partial = e.coverage == "PARTIAL"
        || !w.checked.unresolved.is_empty()
        || scopes.iter().any(|s| {
            s.normalized["inventory"]
                .as_object()
                .into_iter()
                .flatten()
                .any(|(_, f)| {
                    matches!(
                        f["coverage"].as_str(),
                        Some("PARTIAL" | "UNSAFE_FILE" | "AUDIT_UNAVAILABLE")
                    )
                })
        });
    let not_started = entries
        .iter()
        .filter(|e| e["progress"]["status"] == "NOT_STARTED")
        .count();
    let in_progress = entries
        .iter()
        .filter(|e| e["progress"]["status"] == "IN_PROGRESS")
        .count();
    let accounted = entries
        .iter()
        .filter(|e| e["progress"]["status"] == "ACCOUNTED_WITH_BOUNDARIES")
        .count();
    let possible = inventory
        .iter()
        .filter(|r| r["record"]["classification"] == "POSSIBLE_SOURCE_SURFACE")
        .count();
    let unroutable = inventory
        .iter()
        .filter(|r| {
            r["kind"] == "THREAD_ENTRY"
                && r["record"]["rootDeclarations"]
                    .as_array()
                    .is_none_or(|roots| roots.len() != 1)
        })
        .count();
    super::cli::page(
        &digest(&(POLICY, &w.id, &inventory, &p))?,
        entries,
        cursor,
        limit,
        json!({"schema":"codeclew-thread-progress/1.0","policy":POLICY,"work":id,"snapshot":w.snapshot,
        "inventoryStatus":if !scope_known{"UNKNOWN"}else if inventory_partial{"PARTIAL"}else{"SELECTED_SCOPE_RECORDED"},
        "entryCount":inventory.iter().filter(|r|r["kind"]=="THREAD_ENTRY").count(),"possibleSurfaceCount":possible,"unroutableSurfaceCount":unroutable,
        "selectedFileCount":inventory.iter().filter(|r|r["kind"]=="THREAD_FILE").count(),"excludedFileCount":inventory.iter().filter(|r|r["kind"]=="THREAD_EXCLUDED_FILE").count(),
        "unavailableServiceCount":w.checked.unresolved.len(),"notStartedCount":not_started,"inProgressCount":in_progress,"accountedCount":accounted,
        "boundedExplanationAccounted":scope_known&&!inventory_partial&&not_started==0&&in_progress==0&&accounted>0&&unroutable==0&&possible==0,
        "fullScopeComplete":false,"meaningReview":"UNASSESSED","runtimeClosure":"NOT_ESTABLISHED",
        "limitation":"Selected-input inventory and source-bound accounts are separate from universal framework discovery, semantic approval and runtime closure. Unsupported surfaces and unavailable scopes remain visible."}),
    )
}
