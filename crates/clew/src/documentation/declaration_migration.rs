//! Exact compatibility identities across SOURCE and compiler profile selections.
use super::{
    analysis, digest, invalid,
    model::{EXTRACTOR, Observation, SOURCE_EXTRACTOR, ServiceEvidence},
};
use crate::error::ClewError;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

const KIND: &str = "DECLARATION_MIGRATION";
const SCHEMA: &str = "codeclew-documentation-declaration-migration/1.0";
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Migration {
    schema: String,
    source_evidence: ServiceEvidence,
    compiler_evidence: ServiceEvidence,
    declaration_map: BTreeMap<String, String>,
    native_fact_digests: BTreeMap<String, String>,
}

/// Compiler coordinates bind an exported wrapper to the original syntax node.
/// No name-only matching, body rewriting, or offset conversion is admitted.
fn matches(
    source: &super::model::Source,
    native: &super::model::Source,
    owner: &Observation,
) -> bool {
    if owner.normalized["schema"] != crate::typescript_adapter_v2::TYPESCRIPT_FACT_SCHEMA {
        return source.file == native.file
            && source.start_line == native.start_line
            && source.end_line == native.end_line
            && source.text.trim_matches([' ', '\t']) == native.text.trim_matches([' ', '\t']);
    }
    let site = &owner.normalized["outlineOwnerSource"];
    let Some(occurrence) = &source.occurrence else {
        return false;
    };
    let (Some(start), Some(end)) = (site["byteStart"].as_u64(), site["byteEnd"].as_u64()) else {
        return false;
    };
    let Ok(start) = usize::try_from(start) else {
        return false;
    };
    let Ok(end) = usize::try_from(end) else {
        return false;
    };
    if source.file != native.file
        || source.start_line != native.start_line
        || source.end_line != native.end_line
        || site["sourceId"] != native.id
        || site["file"] != native.file
        || site["sourceDigest"] != native.text_digest
        || site["evidenceDigest"] != native.evidence_digest
        || site["sourceStatus"] != "SOURCE_RETAINED"
        || end.checked_sub(start) != Some(native.text.len())
        || occurrence.end_byte != end
        || occurrence.start_byte < start
    {
        return false;
    }
    let offset = occurrence.start_byte - start;
    let Some(prefix) = native.text.get(..offset) else {
        return false;
    };
    matches!(prefix, "" | "export " | "export default ")
        && native.text.get(offset..) == Some(source.text.as_str())
}

fn associations(source: &ServiceEvidence, native: &ServiceEvidence) -> BTreeMap<String, String> {
    let mut index: BTreeMap<(&str, &str, u64, u64), Vec<&Observation>> = BTreeMap::new();
    for o in native.observations.values().filter(|o| {
        o.kind == "SYMBOL"
            && o.normalized["authority"] != "SYNTAX"
            && o.normalized["scope"]
                .as_str()
                .is_some_and(|scope| !scope.is_empty())
    }) {
        let Some(raw_name) = o.normalized["name"].as_str() else {
            continue;
        };
        let name = if raw_name == "<init>" {
            o.normalized["ownerIdentity"]
                .as_str()
                .and_then(|owner| owner.rsplit(['.', '$']).next())
                .unwrap_or(raw_name)
        } else {
            raw_name
        };
        for id in &o.source_ids {
            if let Some(s) = native.sources.get(id) {
                index
                    .entry((name, &s.file, s.start_line, s.end_line))
                    .or_default()
                    .push(o);
            }
        }
    }
    let mut candidates: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut reverse: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for original in source
        .observations
        .values()
        .filter(|o| o.kind == "SYMBOL" && o.normalized["authority"] == "SYNTAX")
    {
        let Some(name) = original.normalized["name"].as_str() else {
            continue;
        };
        for id in &original.source_ids {
            let Some(s) = source.sources.get(id) else {
                continue;
            };
            for o in index
                .get(&(name, &s.file, s.start_line, s.end_line))
                .into_iter()
                .flatten()
            {
                if o.source_ids
                    .iter()
                    .filter_map(|id| native.sources.get(id))
                    .any(|n| matches(s, n, o))
                {
                    candidates
                        .entry(original.id.clone())
                        .or_default()
                        .insert(o.id.clone());
                    reverse
                        .entry(o.id.clone())
                        .or_default()
                        .insert(original.id.clone());
                }
            }
        }
    }
    candidates
        .into_iter()
        .filter_map(|(a, bs)| {
            if bs.len() != 1 {
                return None;
            }
            let b = bs.into_iter().next().unwrap();
            (reverse[&b].len() == 1).then_some((a, b))
        })
        .collect()
}

fn retained(current: &ServiceEvidence) -> Result<Option<Migration>, ClewError> {
    let observations: Vec<_> = current
        .observations
        .values()
        .filter(|o| o.kind == KIND)
        .collect();
    let o = match observations.as_slice() {
        [] => return Ok(None),
        [o] => *o,
        _ => return Err(invalid("ambiguous declaration migration")),
    };
    if o.service != current.service
        || o.digest != digest(&o.normalized)?
        || super::bytes(&o.normalized)?.len() > 32 * 1024 * 1024
    {
        return Err(invalid("invalid declaration migration binding or budget"));
    }
    let m: Migration = serde_json::from_value(o.normalized.clone())
        .map_err(|_| invalid("invalid declaration migration payload"))?;
    let source = &m.source_evidence;
    let native = &m.compiler_evidence;
    if m.schema != SCHEMA
        || source.service != current.service
        || native.service != current.service
        || source.revision != current.revision
        || native.revision != current.revision
        || source.extractor != SOURCE_EXTRACTOR
        || native.extractor != EXTRACTOR
        || [source, native]
            .iter()
            .any(|e| e.observations.values().any(|o| o.kind == KIND))
    {
        return Err(invalid("declaration migration source authority differs"));
    }
    for e in [source, native, current] {
        analysis::verify_evidence(e)?;
    }
    if m.declaration_map != associations(source, native)
        || m.native_fact_digests
            != m.declaration_map
                .values()
                .map(|id| (id.clone(), native.observations[id].digest.clone()))
                .collect()
        || source
            .sources
            .iter()
            .any(|(id, s)| current.sources.get(id) != Some(s))
        || source
            .observations
            .iter()
            .filter(|(_, o)| compatibility_fact(o))
            .any(|(id, o)| current.observations.get(id) != Some(o))
        || (current.extractor == EXTRACTOR
            && (native
                .sources
                .iter()
                .any(|(id, s)| current.sources.get(id) != Some(s))
                || native
                    .observations
                    .iter()
                    .any(|(id, o)| current.observations.get(id) != Some(o))))
    {
        return Err(invalid(
            "declaration migration is not exact retained membership",
        ));
    }
    Ok(Some(m))
}
fn compatibility_fact(o: &Observation) -> bool {
    matches!(
        o.kind.as_str(),
        "SYMBOL" | "FLOW" | "SYNTAX_DETAIL" | "SOURCE_ANNOTATIONS"
    )
}

pub(super) fn attach(
    current: &mut ServiceEvidence,
    previous: &ServiceEvidence,
) -> Result<(), ClewError> {
    if current.service != previous.service || current.revision != previous.revision {
        return Ok(());
    }
    let prior = retained(previous)?;
    let source = if current.extractor == SOURCE_EXTRACTOR {
        current.clone()
    } else if let Some(m) = &prior {
        m.source_evidence.clone()
    } else if previous.extractor == SOURCE_EXTRACTOR {
        previous.clone()
    } else {
        return Ok(());
    };
    let native = if current.extractor == EXTRACTOR {
        current.clone()
    } else if let Some(m) = prior {
        m.compiler_evidence
    } else if previous.extractor == EXTRACTOR {
        previous.clone()
    } else {
        return Ok(());
    };
    for e in [&source, &native] {
        analysis::verify_evidence(e)?;
    }
    let declaration_map = associations(&source, &native);
    let native_fact_digests = declaration_map
        .values()
        .map(|id| (id.clone(), native.observations[id].digest.clone()))
        .collect();
    let migration = Migration {
        schema: SCHEMA.into(),
        source_evidence: source.clone(),
        compiler_evidence: native,
        declaration_map,
        native_fact_digests,
    };
    let normalized = serde_json::to_value(migration).map_err(super::io_error)?;
    if super::bytes(&normalized)?.len() > 32 * 1024 * 1024 {
        return Err(invalid("declaration migration exceeds 32 MiB"));
    }
    for (id, s) in source.sources {
        if current.sources.get(&id).is_some_and(|old| old != &s) {
            return Err(invalid("declaration migration source ID collides"));
        }
        current.sources.insert(id, s);
    }
    for (id, o) in source
        .observations
        .into_iter()
        .filter(|(_, o)| compatibility_fact(o))
    {
        if current.observations.get(&id).is_some_and(|old| old != &o) {
            return Err(invalid("declaration migration fact ID collides"));
        }
        current.observations.insert(id, o);
    }
    let fact_digest = digest(&normalized)?;
    let id = format!(
        "{}:declaration-migration:{}",
        current.service,
        &fact_digest[7..31]
    );
    current.observations.insert(
        id.clone(),
        Observation {
            id,
            kind: KIND.into(),
            service: current.service.clone(),
            symbol: "declaration-migration".into(),
            normalized,
            digest: fact_digest,
            source_ids: vec![],
        },
    );
    current.boundaries.push(
        if current.extractor == SOURCE_EXTRACTOR {
            "SAVED_COMPILER_REFERENCE_IDENTITIES_NOT_CURRENT_COMPILER_AUTHORITY"
        } else {
            "RETAINED_SOURCE_COMPATIBILITY_NOT_RUNTIME_PROOF"
        }
        .into(),
    );
    Ok(())
}

pub(super) fn view(current: &ServiceEvidence) -> Result<Option<ServiceEvidence>, ClewError> {
    let Some(m) = retained(current)? else {
        return Ok(None);
    };
    let mut result = current.clone();
    result.observations.retain(|_, o| o.kind != KIND);
    // Saved citations remain readable after returning to SOURCE. Their original
    // bytes and provenance are retained; no compiler facts enter the SOURCE view.
    if current.extractor == SOURCE_EXTRACTOR {
        for (id, source) in &m.compiler_evidence.sources {
            if result.sources.get(id).is_some_and(|old| old != source) {
                return Err(invalid("saved compiler citation ID collides"));
            }
            result.sources.insert(id.clone(), source.clone());
        }
    }
    if current.extractor == EXTRACTOR {
        for (source_id, native_id) in m.declaration_map {
            let mut o = result.observations.remove(&native_id).unwrap();
            o.id = source_id.clone();
            // Keep the native source windows and byte anchors unchanged.
            result.observations.insert(source_id, o);
        }
    }
    Ok(Some(result))
}
pub(super) fn source(current: &ServiceEvidence) -> Result<Option<ServiceEvidence>, ClewError> {
    Ok(retained(current)?.map(|m| m.source_evidence))
}
pub(super) fn compiler(current: &ServiceEvidence) -> Result<Option<ServiceEvidence>, ClewError> {
    Ok(retained(current)?.map(|m| m.compiler_evidence))
}
pub(super) fn canonical(current: &ServiceEvidence, id: &str) -> Result<String, ClewError> {
    Ok(retained(current)?
        .and_then(|m| {
            m.declaration_map
                .into_iter()
                .find_map(|(source, compiler)| (compiler == id).then_some(source))
        })
        .unwrap_or_else(|| id.into()))
}
pub(super) fn present(evidence: &ServiceEvidence) -> bool {
    evidence.observations.values().any(|o| o.kind == KIND)
}

pub(super) fn native_id(
    current: &ServiceEvidence,
    source_id: &str,
) -> Result<Option<String>, ClewError> {
    Ok(retained(current)?.and_then(|m| m.declaration_map.get(source_id).cloned()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn fixture() -> (ServiceEvidence, ServiceEvidence, String, String) {
        let text = "\u{feff}export function render(): string {\r\n  return 'π🙂'\r\n}";
        let service: super::super::model::Service = serde_json::from_value(json!({"schema":"codeclew-documentation-service/1.0","id":"svc","title":"Owned migration fixture","repositoryId":"svc","repository":"https://example.invalid/test","language":"typescript","profile":"typescript-5-project-read-only","targetRef":"main","compilations":["tsconfig:tsconfig.json"]})).unwrap();
        let mut native: ServiceEvidence = serde_json::from_value(json!({"schema":"codeclew-documentation-service-evidence/1.0","service":"svc","revision":"1".repeat(40),"serviceDigest":"service","extractor":EXTRACTOR,"runtimeMode":"RELEASE","coverage":"COMPILER","boundaries":[],"entrypoints":[],"observations":{},"sources":{},"contracts":{}})).unwrap();
        let source = super::super::syntax::source_for_provider_test(
            &service,
            &native,
            &BTreeMap::from([("Probe.ts".into(), text.into())]),
        );
        let original = source
            .observations
            .values()
            .find(|o| o.kind == "SYMBOL" && o.normalized["name"] == "render")
            .unwrap()
            .id
            .clone();
        let body = &text[3..];
        let source_record: super::super::model::Source = serde_json::from_value(json!({"id":"native-body","service":"svc","revision":"1".repeat(40),"file":"Probe.ts","startLine":1,"endLine":3,"text":body,"textDigest":crate::canonical::hash_bytes(body.as_bytes()),"evidenceDigest":"native-evidence","authority":"EXACT_SNAPSHOT_TEXT"})).unwrap();
        let normalized = json!({"schema":crate::typescript_adapter_v2::TYPESCRIPT_FACT_SCHEMA,"kind":"DECLARATION","declarationKind":"FUNCTION","name":"render","scope":"tsconfig:tsconfig.json","symbolIdentity":"ts:Probe.ts#function:render@3-60","declarationIdentity":"ts:Probe.ts#function:render@3-60","resolution":"COMPILER_RESOLVED","documentation":{"schema":"codeclew-typescript-documentation-flow/1.0","authority":"TYPESCRIPT_COMPILER_SOURCE_STRUCTURE","events":[],"boundaries":[]},"outlineOwnerSource":{"sourceId":"native-body","file":"Probe.ts","sourceStatus":"SOURCE_RETAINED","byteStart":3,"byteEnd":text.len(),"sourceDigest":source_record.text_digest,"evidenceDigest":source_record.evidence_digest}});
        let id = "native-render".to_owned();
        native.observations.insert(
            id.clone(),
            Observation {
                id: id.clone(),
                kind: "SYMBOL".into(),
                service: "svc".into(),
                symbol: "ts:Probe.ts#function:render@3-60".into(),
                digest: digest(&normalized).unwrap(),
                normalized,
                source_ids: vec![source_record.id.clone()],
            },
        );
        native
            .sources
            .insert(source_record.id.clone(), source_record);
        (source, native, original, id)
    }
    #[test]
    fn exported_original_byte_aliases_work_in_both_modes_without_source_compiler_authority() {
        let (source, mut native, original, compiler_id) = fixture();
        attach(&mut native, &source).unwrap();
        assert_eq!(canonical(&native, &compiler_id).unwrap(), original);
        let compiled = view(&native).unwrap().unwrap();
        assert_eq!(
            compiled.observations[&original].normalized["scope"],
            "tsconfig:tsconfig.json"
        );
        assert_eq!(
            compiled.observations[&original].source_ids,
            vec!["native-body"]
        );
        assert!(!compiled.observations.contains_key(&compiler_id));
        let mut restored = source.clone();
        attach(&mut restored, &native).unwrap();
        assert_eq!(canonical(&restored, &compiler_id).unwrap(), original);
        let syntax = view(&restored).unwrap().unwrap();
        assert_eq!(syntax.extractor, SOURCE_EXTRACTOR);
        assert_eq!(syntax.sources["native-body"], native.sources["native-body"]);
        assert_eq!(
            syntax.observations[&original],
            source.observations[&original]
        );
        assert!(
            !syntax
                .observations
                .values()
                .any(|o| o.symbol.starts_with("ts:"))
        );
    }
    #[test]
    fn ambiguous_scopes_changed_revision_content_and_offsets_do_not_create_aliases() {
        let (source, native, original, compiler_id) = fixture();
        for case in ["scope", "bytes", "offset", "prefix", "revision"] {
            let mut changed = native.clone();
            match case {
                "scope" => {
                    let mut second = changed.observations[&compiler_id].clone();
                    second.id = "second".into();
                    second.normalized["scope"] = json!("tsconfig:other.json");
                    second.digest = digest(&second.normalized).unwrap();
                    changed.observations.insert(second.id.clone(), second);
                }
                "bytes" | "prefix" => {
                    let s = changed.sources.get_mut("native-body").unwrap();
                    s.text = if case == "bytes" {
                        s.text.replace("π🙂", "other")
                    } else {
                        s.text.replace("export ", "delete ")
                    };
                    s.text_digest = crate::canonical::hash_bytes(s.text.as_bytes());
                    let o = changed.observations.get_mut(&compiler_id).unwrap();
                    o.normalized["outlineOwnerSource"]["sourceDigest"] = json!(s.text_digest);
                    o.normalized["outlineOwnerSource"]["byteEnd"] = json!(3 + s.text.len());
                    o.digest = digest(&o.normalized).unwrap();
                }
                "offset" => {
                    let o = changed.observations.get_mut(&compiler_id).unwrap();
                    o.normalized["outlineOwnerSource"]["byteStart"] = json!(4);
                    o.digest = digest(&o.normalized).unwrap();
                }
                "revision" => {
                    changed.revision = "2".repeat(40);
                    for s in changed.sources.values_mut() {
                        s.revision = changed.revision.clone();
                    }
                }
                _ => unreachable!(),
            }
            attach(&mut changed, &source).unwrap();
            assert_eq!(
                canonical(&changed, &compiler_id).unwrap(),
                compiler_id,
                "{case}"
            );
            if let Some(view) = view(&changed).unwrap() {
                assert_eq!(
                    view.observations[&original].normalized["authority"],
                    "SYNTAX"
                );
            }
        }
    }
    #[test]
    fn migration_rejects_rehashed_target_corruption() {
        let (source, mut native, _, compiler_id) = fixture();
        attach(&mut native, &source).unwrap();
        native
            .observations
            .get_mut(&compiler_id)
            .unwrap()
            .normalized["name"] = json!("unrelated");
        let o = native.observations.get_mut(&compiler_id).unwrap();
        o.digest = digest(&o.normalized).unwrap();
        assert!(view(&native).is_err());
    }
}
