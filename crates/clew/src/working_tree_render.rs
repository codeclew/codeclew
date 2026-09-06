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
    let (_, report) = working_tree_change_service::load(id)?;
    let state = StateAuthority::process_default()?;
    let store = CasStore::open(&state)?;
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
    let (_, report) = working_tree_change_service::load(id)?;
    let state = StateAuthority::process_default()?;
    let store = CasStore::open(&state)?;
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
    let (_, report) = working_tree_change_service::load(id)?;
    let state = StateAuthority::process_default()?;
    let store = CasStore::open(&state)?;
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
