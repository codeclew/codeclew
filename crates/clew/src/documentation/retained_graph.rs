//! Bounded retained source-call/data context. No documentation publication.
pub(in crate::documentation) mod data_state;
pub(in crate::documentation) mod linked;
pub mod model;
pub(in crate::documentation) mod source;
use super::{check::Check, invalid};
use crate::error::ClewError;

/// Callable-only projection used by opt-in immutable model Work. Unlike pages,
/// it makes no endpoint/worker/handoff association and selects no live state.
pub(super) fn source_data_graph(
    checked: &Check,
    service: &str,
    declaration: &str,
) -> Result<model::SourceCallGraph, ClewError> {
    source_data_graph_with_depth(checked, service, declaration, 2)
}

pub(super) fn source_data_graph_with_depth(
    checked: &Check,
    service: &str,
    declaration: &str,
    max_depth: usize,
) -> Result<model::SourceCallGraph, ClewError> {
    let e = checked
        .services
        .get(service)
        .ok_or_else(|| invalid("source-data service is unavailable"))?;
    let d = e
        .observations
        .get(declaration)
        .ok_or_else(|| invalid("source-data root declaration is unavailable"))?;
    if !data_state::supported_language(d) {
        return Err(invalid(
            "sourceDataContext requires an exact retained Java or Kotlin compiler callable",
        ));
    }
    let root_bytes: usize = d
        .source_ids
        .iter()
        .filter_map(|id| e.sources.get(id))
        .map(|s| s.text.len())
        .sum();
    if root_bytes > 1024 * 1024 {
        return Err(invalid(
            "sourceDataContext root exceeds 1 MiB before parsing; narrow the selection",
        ));
    }
    let mut graph = linked::build_roots_with_depth(
        checked,
        &[(service.into(), declaration.into())],
        max_depth,
    )?;
    for node in graph.nodes.values_mut() {
        node.examined_source_digest = linked::node_digest(node)?;
    }
    let selected = graph.nodes.keys().cloned().collect();
    data_state::attach_graph(checked, &mut graph, &selected)?;
    Ok(graph)
}
