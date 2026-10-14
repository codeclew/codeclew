//! Retained native evidence behind unchanged SOURCE declarations.
use super::{analysis, check::Check, digest, invalid, model::ServiceEvidence};
use crate::error::ClewError;
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Overlay {
    schema: String,
    evidence: ServiceEvidence,
    declaration_map: BTreeMap<String, String>,
}

fn declaration_bytes(text: &str) -> &str {
    text.trim_matches([' ', '\t'])
}

/// Equal declaration bytes and ranges must identify exactly one declaration on each side.
/// Compiler line captures may include horizontal padding around the exact syntax node.
/// File/range indexes keep admission bounded by retained records, not all pairs.
pub(super) fn associations(
    source: &ServiceEvidence,
    native: &ServiceEvidence,
) -> BTreeMap<String, String> {
    type Key = (String, String, u64, u64, String);
    let index =
        |evidence: &ServiceEvidence, syntax: bool| {
            let mut rows: BTreeMap<Key, BTreeSet<String>> = BTreeMap::new();
            for o in evidence.observations.values().filter(|o| {
                o.kind == "SYMBOL" && (!syntax || o.normalized["authority"] == "SYNTAX")
            }) {
                let Some(name) = o.normalized["name"].as_str() else {
                    continue;
                };
                for id in &o.source_ids {
                    if let Some(s) = evidence.sources.get(id) {
                        rows.entry((
                            name.into(),
                            s.file.clone(),
                            s.start_line,
                            s.end_line,
                            crate::canonical::hash_bytes(declaration_bytes(&s.text).as_bytes()),
                        ))
                        .or_default()
                        .insert(o.id.clone());
                    }
                }
            }
            rows
        };
    let originals = index(source, true);
    let natives = index(native, false);
    let mut candidates: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut reverse: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut blocked_source = BTreeSet::new();
    let mut blocked_native = BTreeSet::new();
    for (key, ids) in originals {
        let Some(others) = natives.get(&key) else {
            continue;
        };
        // Multiple declarations for one retained range are an explicit boundary.
        if ids.len() != 1 || others.len() != 1 {
            blocked_source.extend(ids);
            blocked_native.extend(others.iter().cloned());
            continue;
        }
        let a = ids.into_iter().next().unwrap();
        let b = others.iter().next().unwrap().clone();
        candidates.entry(a.clone()).or_default().insert(b.clone());
        reverse.entry(b).or_default().insert(a);
    }
    candidates
        .into_iter()
        .filter_map(|(a, bs)| {
            if bs.len() != 1 {
                return None;
            }
            let b = bs.into_iter().next().unwrap();
            (reverse[&b].len() == 1 && !blocked_source.contains(&a) && !blocked_native.contains(&b))
                .then_some((a, b))
        })
        .collect()
}

/// Ephemeral consumer bindings are derived from the immutable overlay dependency.
/// Never store this view as a new capture or replace SOURCE producer authority.
pub(super) fn checked_view(checked: &Check) -> Result<Check, ClewError> {
    let mut result = checked.clone();
    for (service, original) in &checked.services {
        let Some(evidence) = view(original)? else {
            continue;
        };
        for o in original.observations.values() {
            if !checked.dependencies.get(&o.id).is_some_and(|d| {
                d.digest == o.digest
                    && d.normalized == o.normalized
                    && d.service == o.service
                    && d.symbol == o.symbol
                    && d.source_ids == o.source_ids
            }) {
                return Err(invalid(
                    "compiler overlay does not bind retained Check dependencies",
                ));
            }
        }
        for (id, o) in &evidence.observations {
            if let Some(existing) = result.dependencies.get(id)
                && existing.service != *service
            {
                return Err(invalid("compiler overlay dependency ID collides"));
            }
            result.dependencies.insert(id.clone(), o.clone());
        }
        result.services.insert(service.clone(), evidence);
    }
    Ok(result)
}

/// Build a consumer view; the stored SOURCE observations and citations remain
/// immutable. Compiler symbols remain native, while mapped declaration IDs are
/// the existing SOURCE IDs. Unmapped native declarations retain their own IDs.
pub(super) fn view(source: &ServiceEvidence) -> Result<Option<ServiceEvidence>, ClewError> {
    let overlays: Vec<_> = source
        .observations
        .values()
        .filter(|o| o.kind == "SEMANTIC_OVERLAY")
        .collect();
    let overlay = match overlays.as_slice() {
        [] => return super::declaration_migration::view(source),
        [overlay] => *overlay,
        _ => return Err(invalid("ambiguous retained compiler overlay")),
    };
    analysis::verify_evidence(source)?;
    if super::bytes(&overlay.normalized)?.len() > 32 * 1024 * 1024 {
        return Err(invalid("retained compiler overlay exceeds 32 MiB"));
    }
    if overlay.service != source.service || digest(&overlay.normalized)? != overlay.digest {
        return Err(invalid(
            "retained compiler overlay digest or service differs",
        ));
    }
    let retained: Overlay = serde_json::from_value(overlay.normalized.clone())
        .map_err(|_| invalid("invalid retained compiler overlay"))?;
    if retained.schema != "codeclew-documentation-semantic-overlay/1.0"
        || retained.evidence.service != source.service
        || retained.evidence.revision != source.revision
    {
        return Err(invalid(
            "retained compiler overlay revision or service differs",
        ));
    }
    analysis::verify_evidence(&retained.evidence)?;
    if retained.evidence.observations.len() > 131_072
        || retained.evidence.sources.len() > 131_072
        || retained
            .evidence
            .observations
            .values()
            .any(|o| o.kind == "SEMANTIC_OVERLAY")
        || retained.declaration_map != associations(source, &retained.evidence)
    {
        return Err(invalid(
            "compiler overlay associations are not the exact unique retained set",
        ));
    }
    let mut targets = BTreeSet::new();
    for (source_id, native_id) in &retained.declaration_map {
        let original = source
            .observations
            .get(source_id)
            .ok_or_else(|| invalid("compiler overlay SOURCE declaration is missing"))?;
        let native = retained
            .evidence
            .observations
            .get(native_id)
            .ok_or_else(|| invalid("compiler overlay native declaration is missing"))?;
        if original.kind != "SYMBOL"
            || native.kind != "SYMBOL"
            || original.normalized["authority"] != "SYNTAX"
            || original.normalized["name"] != native.normalized["name"]
            || !targets.insert(native_id.clone())
            || !original.source_ids.iter().any(|id| {
                native.source_ids.iter().any(|other| {
                    let Some(a) = source.sources.get(id) else {
                        return false;
                    };
                    let Some(b) = retained.evidence.sources.get(other) else {
                        return false;
                    };
                    a.file == b.file
                        && a.start_line == b.start_line
                        && a.end_line == b.end_line
                        && declaration_bytes(&a.text) == declaration_bytes(&b.text)
                })
            })
        {
            return Err(invalid(
                "compiler overlay declaration association is not exact and unique",
            ));
        }
    }
    let mut result = retained.evidence;
    for (source_id, native_id) in retained.declaration_map {
        let mut native = result.observations.remove(&native_id).unwrap();
        let original = &source.observations[&source_id];
        // The declaration body must be identical for the exact mapping above.
        // Preserve its original citation record rather than rebinding the
        // accepted SOURCE citation to the compiler's capture occurrence.
        native.source_ids = original.source_ids.clone();
        native.id = source_id.clone();
        if result.observations.insert(source_id, native).is_some() {
            return Err(invalid("compiler overlay declaration ID collides"));
        }
    }
    for (id, record) in &source.sources {
        if let Some(existing) = result.sources.get(id)
            && existing != record
        {
            return Err(invalid("compiler and SOURCE citation IDs collide"));
        }
        result.sources.insert(id.clone(), record.clone());
    }
    for (id, record) in &source.observations {
        if matches!(
            record.kind.as_str(),
            "SYMBOL" | "FLOW" | "SYNTAX_DETAIL" | "SOURCE_ANNOTATIONS"
        ) && !result.observations.contains_key(id)
        {
            result.observations.insert(id.clone(), record.clone());
        }
    }
    result.service_digest = source.service_digest.clone();
    Ok(Some(result))
}
