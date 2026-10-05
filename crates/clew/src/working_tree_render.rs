//! Offline, retained change explanations. Rendering never resolves a live model.
use crate::canonical;
use crate::cas::CasStore;
use crate::error::{ClewError, ErrorCode};
use crate::explanation::ClaimAuthority;
use crate::explanation_freshness::FreshnessStatus;
use crate::repository_snapshot::{self, RepositoryInputSnapshot};
use crate::state::StateAuthority;
use crate::working_tree_change::{Comparison, SourceAnchor};
use crate::working_tree_change_service;
use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::io::Write;
use std::path::Path;

const WINDOW_BYTES: usize = 16 * 1024;
const EMBEDDED_BYTES: usize = 8 * 1024 * 1024;
const HTML_BYTES: usize = 16 * 1024 * 1024;

fn error(code: ErrorCode, message: impl std::fmt::Display) -> ClewError {
    ClewError::new(code, message.to_string())
}
fn invalid(message: &str) -> ClewError {
    error(ErrorCode::InvalidInput, message)
}

fn window(
    store: &CasStore,
    anchor: &SourceAnchor,
    offset: usize,
    limit: usize,
) -> Result<Value, ClewError> {
    let text = anchor.text(store)?;
    if offset > text.len() || !text.is_char_boundary(offset) {
        return Err(invalid(
            "source offset must be a UTF-8 boundary within the retained anchor",
        ));
    }
    let mut end = text.len().min(offset.saturating_add(limit));
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    Ok(
        json!({"anchor":anchor,"text":&text[offset..end],"offset":offset,
        "start":anchor.start+offset,"end":anchor.start+end,"totalBytes":text.len(),
        "nextOffset":(end < text.len()).then_some(end),"authority":"EXACT_RETAINED_SOURCE"}),
    )
}

pub fn source(
    id: &str,
    node: Option<&str>,
    file: Option<&str>,
    side: &str,
    offset: usize,
    limit: usize,
) -> Result<Value, ClewError> {
    if !(1..=WINDOW_BYTES).contains(&limit) {
        return Err(invalid("source limit must be 1..16384 bytes"));
    }
    let state = StateAuthority::process_default()?;
    source_with_state(&state, id, node, file, side, offset, limit)
}

fn source_with_state(
    state: &StateAuthority,
    id: &str,
    node: Option<&str>,
    file: Option<&str>,
    side: &str,
    offset: usize,
    limit: usize,
) -> Result<Value, ClewError> {
    let store = CasStore::open(state)?;
    let (_, report) = working_tree_change_service::load_with_store(state, &store, id)?;
    let selected = match side {
        "before" => &report.before,
        "after" => &report.after,
        _ => return Err(invalid("side must be before or after")),
    };
    let anchor = if let Some(node_id) = node {
        let node = report
            .consequences
            .as_ref()
            .and_then(|g| g.nodes.iter().find(|n| n.node_id == node_id))
            .ok_or_else(|| invalid("node not retained in comparison"))?;
        let declaration = if side == "before" {
            &node.before
        } else {
            &node.after
        };
        declaration
            .as_ref()
            .ok_or_else(|| invalid("declaration absent on selected side"))?
            .source
            .clone()
    } else if let Some(file) = file {
        let lease = store.read(
            &selected.snapshot,
            crate::working_tree_change::MAX_REPORT_BYTES,
        )?;
        let snapshot: RepositoryInputSnapshot =
            serde_json::from_slice(lease.bytes()).map_err(|e| error(ErrorCode::Internal, e))?;
        let files = crate::working_tree_change::source_files(&snapshot);
        let content = files
            .get(file)
            .ok_or_else(|| invalid("file absent from selected snapshot"))?
            .content
            .clone();
        let bytes = store.read(
            &content,
            repository_snapshot::WorkingTreeLimits::default().max_file_bytes as usize,
        )?;
        let text =
            std::str::from_utf8(bytes.bytes()).map_err(|_| invalid("source is not UTF-8"))?;
        SourceAnchor {
            file: file.into(),
            start: 0,
            end: content.size as usize,
            start_line: 1,
            end_line: 1 + text.bytes().filter(|b| *b == b'\n').count(),
            content,
        }
    } else {
        return Err(invalid("select a node or file"));
    };
    let mut result = window(&store, &anchor, offset, limit)?;
    result["schema"] = json!("codeclew-change-source/1.0");
    result["comparisonId"] = json!(id);
    result["side"] = json!(side);
    result["snapshot"] = json!(selected.snapshot);
    Ok(result)
}

pub fn freshness(id: &str) -> Result<Value, ClewError> {
    let state = StateAuthority::process_default()?;
    freshness_with_state(&state, id)
}

fn freshness_with_state(state: &StateAuthority, id: &str) -> Result<Value, ClewError> {
    let store = CasStore::open(state)?;
    let (_, report) = working_tree_change_service::load_with_store(state, &store, id)?;
    // Check the source and fact objects used by claims, not just the report hash.
    let mut checked = BTreeSet::new();
    for side in [&report.before, &report.after] {
        let lease = store.read(&side.snapshot, crate::working_tree_change::MAX_REPORT_BYTES)?;
        let snapshot: RepositoryInputSnapshot =
            serde_json::from_slice(lease.bytes()).map_err(|e| error(ErrorCode::Internal, e))?;
        for object in snapshot
            .index
            .iter()
            .map(|e| &e.content)
            .chain(snapshot.worktree.iter().filter_map(|e| e.content.as_ref()))
            .chain(side.analysis.declarations.iter().map(|d| &d.payload))
            .chain(side.analysis.relations.iter().map(|r| &r.payload))
        {
            if checked.insert(object.digest.clone()) {
                store.read(object, crate::working_tree_change::MAX_REPORT_BYTES)?;
            }
        }
    }
    let live = report
        .after
        .session
        .target_repository_path()
        .and_then(|repository| repository_snapshot::capture_working_tree(&repository, &store));
    let matches = live.as_ref().ok().map(|(head, _, snapshot)| {
        head == &report.after.session.base_revision && snapshot == &report.after.snapshot
    });
    Ok(
        json!({"schema":"codeclew-change-freshness/1.0","comparisonId":id,"retainedEvidenceValid":true,
        "retainedEvidenceScope":"COMPARISON_AND_CLAIM_SOURCE_AND_FACT_OBJECTS","liveSnapshotMatches":matches,
        "liveStatus":match matches {Some(true)=>"FRESH",Some(false)=>"LIVE_CHANGED",None=>"LIVE_UNAVAILABLE"},
        "liveFailure":live.err().map(|e| json!({"code":e.code,"message":e.message})),
        "coverage":coverage(&report),"nextAction":if matches==Some(true){"NONE"}else{"CAPTURE_NEW_COMPARISON_IF_CURRENT_EDITS_ARE_NEEDED"}}),
    )
}

fn coverage(report: &Comparison) -> Value {
    json!({"status":report.status,"compilations":report.after.session.compilations,
        "beforeDeclarationsComplete":report.before.analysis.declaration_coverage_complete,
        "afterDeclarationsComplete":report.after.analysis.declaration_coverage_complete,
        "beforeAnalysis":report.before.analysis.status,"afterAnalysis":report.after.analysis.status,
        "beforeFailure":report.before.analysis.failure,"afterFailure":report.after.analysis.failure,
        "beforeBoundaries":working_tree_change_service::boundary_summary(&report.before.analysis),
        "afterBoundaries":working_tree_change_service::boundary_summary(&report.after.analysis),
        "comparability":report.comparability,"obligations":report.obligations,"testsExecuted":false})
}

fn model(store: &CasStore, report: &Comparison) -> Result<Value, ClewError> {
    let graph = report
        .consequences
        .as_ref()
        .ok_or_else(|| invalid("comparison predates retained consequences; inspect again"))?;
    let mut claims = Vec::new();
    let mut sources = serde_json::Map::new();
    let mut remaining = EMBEDDED_BYTES;
    for node in &graph.nodes {
        let change = node
            .change_id
            .as_ref()
            .and_then(|id| report.declarations.iter().find(|d| &d.change_id == id));
        let statement = match change {
            Some(c) if c.source_text_changed => {
                "The retained declaration text changed. Behavioral change is not proven by text alone."
            }
            Some(_) => {
                "The retained declaration has a recorded identity, location or projected-shape change."
            }
            None => {
                "This directly connected declaration retains its own before/after evidence. Unchanged text does not prove it is unaffected at runtime."
            }
        };
        claims.push(json!({"claimId":node.node_id,"kind":"DECLARATION_OBSERVATION","authority":ClaimAuthority::StaticDerived,
            "statement":statement,"change":change,"before":node.before.as_ref().map(|d| &d.source),"after":node.after.as_ref().map(|d| &d.source),
            "beforeSnapshot":report.before.snapshot,"afterSnapshot":report.after.snapshot}));
        let mut sides = serde_json::Map::new();
        for (side, d) in [("before", &node.before), ("after", &node.after)] {
            if let Some(d) = d {
                let result = window(store, &d.source, 0, remaining.min(WINDOW_BYTES))?;
                remaining = remaining.saturating_sub(result["text"].as_str().unwrap().len());
                sides.insert(side.into(), result);
            }
        }
        sources.insert(node.node_id.clone(), Value::Object(sides));
    }
    for edge in &graph.edges {
        let status = match edge.claim_freshness.as_str() {
            "DIRECT_RELATION_PRESERVED_NOT_BEHAVIORAL_EQUIVALENCE"
            | "NEWLY_OBSERVED_DIRECT_RELATION" => FreshnessStatus::Current,
            "BEFORE_RELATION_CLAIM_STALE_FOR_AFTER" => FreshnessStatus::Stale,
            "NOT_OBSERVED_AFTER_WITH_PARTIAL_COVERAGE" => FreshnessStatus::Unresolved,
            _ => FreshnessStatus::Unresolved,
        };
        claims.push(json!({"claimId":edge.edge_id,"kind":"DIRECT_RELATION","authority":ClaimAuthority::CompilerProven,
            "statement":"Compiler relation in the recorded side and supported subset; no runtime failure or execution order is implied.",
            "freshness":status,"before":edge.before,"after":edge.after,"beforeSnapshot":report.before.snapshot,"afterSnapshot":report.after.snapshot}));
    }
    Ok(
        json!({"schema":"codeclew-change-explanation/1.0","comparisonId":report.comparison_id,
        "baseRevision":report.before.session.base_revision,"beforeSnapshot":report.before.snapshot,"afterSnapshot":report.after.snapshot,
        "narrativeAuthority":"DETERMINISTIC_EVIDENCE_LABELS","liveStatus":"NOT_CHECKED_BY_OFFLINE_RENDER",
        "coverage":coverage(report),"graph":graph,"claims":claims,"sources":sources,"files":report.files,
        "counts":{"files":report.total_changed_file_count,"declarations":report.total_changed_declaration_count,"unchangedDeclarations":report.unchanged_declaration_count,
            "omittedFiles":report.omitted_file_count,"omittedDeclarations":report.omitted_declaration_count},
        "sourceBudgetBytes":EMBEDDED_BYTES,"embeddedSourceBytes":EMBEDDED_BYTES-remaining}),
    )
}

fn html(value: &Value) -> Result<String, ClewError> {
    let data = serde_json::to_string(value)
        .map_err(|e| error(ErrorCode::Internal, e))?
        .replace('<', "\\u003c")
        .replace('&', "\\u0026");
    let output =
        include_str!("working_tree_report.html").replace("__CODECLEW_CHANGE_DATA__", &data);
    if output.len() > HTML_BYTES {
        return Err(error(
            ErrorCode::ResourceLimit,
            "local report exceeds 16 MiB; use bounded show, graph and source commands",
        ));
    }
    Ok(output)
}

pub fn render(id: &str, output: &Path) -> Result<Value, ClewError> {
    let state = StateAuthority::process_default()?;
    render_with_state(&state, id, output)
}

fn render_with_state(state: &StateAuthority, id: &str, output: &Path) -> Result<Value, ClewError> {
    let store = CasStore::open(state)?;
    let (_, report) = working_tree_change_service::load_with_store(state, &store, id)?;
    let value = model(&store, &report)?;
    let html = html(&value)?;
    let parent = output
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let mut temporary =
        tempfile::NamedTempFile::new_in(parent).map_err(|e| error(ErrorCode::InvalidInput, e))?;
    temporary
        .write_all(html.as_bytes())
        .map_err(|e| error(ErrorCode::Internal, e))?;
    temporary
        .persist_noclobber(output)
        .map_err(|e| error(ErrorCode::InvalidInput, e))?;
    Ok(
        json!({"schema":"codeclew-change-render/1.0","comparisonId":id,"output":output.canonicalize().map_err(|e|error(ErrorCode::Internal,e))?,
        "bytes":html.len(),"contentDigest":canonical::hash(&html).map_err(|e|error(ErrorCode::Internal,e))?,
        "status":"RENDERED","compilerExecuted":false,"liveStatus":"NOT_CHECKED_BY_OFFLINE_RENDER"}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repository_snapshot::IndexEntry;
    use crate::runtime::RuntimeMode;
    use crate::session::{
        ModelCachePolicy, SessionAuthority, SessionLanguage, WorkingTreeSourceBinding,
    };
    use crate::working_tree_change::{self as change, Analysis, Side};

    #[test]
    fn retained_commands_admit_once_and_source_preserves_exact_bytes() {
        let temporary = tempfile::tempdir().unwrap();
        let state = StateAuthority::open(temporary.path().join("v2")).unwrap();
        let store = CasStore::open(&state).unwrap();
        let before_text = "fun price() = \"λ before\"\n";
        let after_text = "fun price() = \"λ saved\"\n";
        let snapshot = |text: &str| {
            let content = store.put("test/source/1", text.as_bytes()).unwrap();
            let snapshot = RepositoryInputSnapshot {
                schema: repository_snapshot::SNAPSHOT_SCHEMA.into(),
                snapshot_id: String::new(),
                staged_view_digest: String::new(),
                cached_view_digest: String::new(),
                untracked_view_digest: String::new(),
                index: vec![IndexEntry {
                    path: "Price.kt".into(),
                    mode: 0o100644,
                    stage: 0,
                    git_oid: "0".repeat(40),
                    content,
                }],
                worktree: vec![],
            };
            let object = store
                .put(
                    repository_snapshot::SNAPSHOT_SCHEMA,
                    &canonical::bytes(&snapshot).unwrap(),
                )
                .unwrap();
            (snapshot, object)
        };
        let (before_snapshot, before_object) = snapshot(before_text);
        let (after_snapshot, after_object) = snapshot(after_text);
        let session = SessionAuthority {
            schema: "test".into(),
            authority_digest: String::new(),
            session_id: String::new(),
            repository_key: "fixture".into(),
            base_revision: "0".repeat(40),
            target_ref: "refs/heads/main".into(),
            target_oid: "0".repeat(40),
            runtime_key: String::new(),
            runtime_mode: RuntimeMode::Development,
            language: SessionLanguage::Kotlin,
            compilations: vec![":/main".into()],
            generation_jobs: None,
            model_cache_policy: ModelCachePolicy::NonCacheable,
            model_cache_authority: None,
            maven_settings_digest: None,
            profile: None,
            working_tree: None,
            created_unix_ms: 0,
        };
        let before = Side {
            profile_id: "test".into(),
            session,
            snapshot: before_object,
            analysis: Analysis {
                status: "UNAVAILABLE".into(),
                authority: "TEST".into(),
                ready: None,
                declarations: vec![],
                relations: vec![],
                relation_coverage_complete: false,
                declaration_coverage_complete: false,
                boundaries: vec![],
                failure: None,
            },
        };
        let mut after = before.clone();
        after.snapshot = after_object;
        after.session.working_tree = Some(WorkingTreeSourceBinding {
            schema: "test".into(),
            source_selection: "WORKING_TREE".into(),
            operation: "ANALYSIS".into(),
            profile_id: "test".into(),
            snapshot: after.snapshot.clone(),
            capture_scope: "test".into(),
            excluded_categories: vec![],
            consistency: "test".into(),
            limits: repository_snapshot::WorkingTreeLimits::default(),
        });
        let mut report =
            change::compare(&store, before, after, &before_snapshot, &after_snapshot).unwrap();
        report.consequences = Some(crate::working_tree_consequences::build(&report).unwrap());
        report.seal().unwrap();
        let root = working_tree_change_service::ChangeRoot {
            schema: "codeclew-working-tree-change-root/1.0".into(),
            comparison_id: report.comparison_id.clone(),
            report: store
                .put(change::SCHEMA, &canonical::bytes(&report).unwrap())
                .unwrap(),
        };
        let changes = state.directory(Path::new("changes")).unwrap();
        let component = report
            .comparison_id
            .strip_prefix("comparison:sha256:")
            .unwrap();
        state
            .write_private_atomic(
                &changes.path().join(format!("{component}.json")),
                &canonical::bytes(&root).unwrap(),
            )
            .unwrap();
        // Drop the fixture store so command admission cannot borrow its shared catalog.
        drop(store);
        for (side, expected) in [("before", before_text), ("after", after_text)] {
            let admissions = crate::cas::catalog_admissions_for_current_thread();
            let source = source_with_state(
                &state,
                &report.comparison_id,
                None,
                Some("Price.kt"),
                side,
                0,
                WINDOW_BYTES,
            )
            .unwrap();
            assert_eq!(
                crate::cas::catalog_admissions_for_current_thread() - admissions,
                1
            );
            assert_eq!(source["text"], expected);
            assert_eq!(source["totalBytes"], expected.len());
            assert_eq!(source["authority"], "EXACT_RETAINED_SOURCE");
        }
        let admissions = crate::cas::catalog_admissions_for_current_thread();
        let rendered = render_with_state(
            &state,
            &report.comparison_id,
            &temporary.path().join("report.html"),
        )
        .unwrap();
        assert_eq!(
            crate::cas::catalog_admissions_for_current_thread() - admissions,
            1
        );
        assert_eq!(rendered["status"], "RENDERED");
        let admissions = crate::cas::catalog_admissions_for_current_thread();
        let freshness = freshness_with_state(&state, &report.comparison_id).unwrap();
        assert_eq!(
            crate::cas::catalog_admissions_for_current_thread() - admissions,
            1
        );
        assert_eq!(freshness["retainedEvidenceValid"], true);
    }

    #[test]
    fn source_cannot_escape_embedded_json_or_become_html() {
        let value = json!({"source":"</script><img src=x onerror=alert(1)>&"});
        let rendered = html(&value).unwrap();
        assert!(!rendered.contains("</script><img"));
        assert!(rendered.contains("\\u003c/script>"));
        assert!(!rendered.contains("innerHTML"));
        assert_eq!(rendered, html(&value).unwrap());
    }
}
