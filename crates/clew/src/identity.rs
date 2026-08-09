//! Conservative declaration identity decisions between semantic-index snapshots.
//!
//! The extractor never guesses continuity. Exact symbol identities survive
//! unchanged or moved source origins; structural matches are accepted only
//! when they are unique in both snapshots. All one-to-many, many-to-one and
//! decoy cases remain explicitly ambiguous.

use crate::canonical;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use thiserror::Error;

pub const IDENTITY_REPORT_SCHEMA: &str = "identity-report/0.1";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotProvenance {
    pub composite_snapshot_hash: String,
    pub index_snapshot_hash: String,
    pub project_model_hash: String,
    pub classpath_hash: String,
    pub compiler_options_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum IdentityLifecycle {
    Same,
    Renamed,
    Moved,
    Split,
    Merged,
    Deleted,
    Ambiguous,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum IdentityConfidence {
    Exact,
    Strong,
    Ambiguous,
    None,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceFactProvenance {
    pub file: String,
    pub content_hash: String,
    pub range_start: u64,
    pub range_end: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeclarationIdentity {
    pub declaration_id: String,
    pub symbol_id: String,
    pub name: String,
    pub kind: String,
    pub containing_declaration: Option<String>,
    pub source: SourceFactProvenance,
    pub source_signature_hash: String,
    pub body_hash: String,
    pub abi_hash: String,
    pub semantic_summary_hash: String,
    pub identity_shape_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FieldDelta {
    pub before: Option<String>,
    pub after: Option<String>,
    pub changed: bool,
}

impl FieldDelta {
    fn between(before: impl Into<Option<String>>, after: impl Into<Option<String>>) -> Self {
        let before = before.into();
        let after = after.into();
        let changed = before != after;
        Self {
            before,
            after,
            changed,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeclarationFactDelta {
    pub declaration_id: FieldDelta,
    pub symbol_id: FieldDelta,
    pub kind: FieldDelta,
    pub containing_declaration: FieldDelta,
    pub source_origin: FieldDelta,
    pub source_signature_hash: FieldDelta,
    pub body_hash: FieldDelta,
    pub abi_hash: FieldDelta,
    pub semantic_summary_hash: FieldDelta,
    pub identity_shape_hash: FieldDelta,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BuildModelFactDelta {
    pub project_model_hash: FieldDelta,
    pub classpath_hash: FieldDelta,
    pub compiler_options_hash: FieldDelta,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IdentityDecision {
    pub lifecycle: IdentityLifecycle,
    pub confidence: IdentityConfidence,
    pub before: Vec<DeclarationIdentity>,
    pub after: Vec<DeclarationIdentity>,
    pub candidates: Vec<String>,
    pub fact_delta: Option<DeclarationFactDelta>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IdentityReport {
    pub schema: String,
    pub before: SnapshotProvenance,
    pub after: SnapshotProvenance,
    pub build_model_delta: BuildModelFactDelta,
    pub decisions: Vec<IdentityDecision>,
    pub introduced: Vec<DeclarationIdentity>,
    pub supported_contour: Vec<IdentityLifecycle>,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum IdentityError {
    #[error("snapshot provenance field {0} is empty")]
    EmptySnapshotField(&'static str),
    #[error("cannot canonicalize identity shape: {0}")]
    Canonical(String),
}

pub fn decide_identity_delta(
    before: SnapshotProvenance,
    after: SnapshotProvenance,
    before_files: &[Value],
    after_files: &[Value],
) -> Result<IdentityReport, IdentityError> {
    validate_snapshot(&before)?;
    validate_snapshot(&after)?;
    let old = declarations(before_files)?;
    let new = declarations(after_files)?;
    let mut old_unmatched: BTreeSet<usize> = (0..old.len()).collect();
    let mut new_unmatched: BTreeSet<usize> = (0..new.len()).collect();
    let mut decisions = Vec::new();

    let old_symbols = group_indices(&old, |item| item.symbol_id.clone());
    let new_symbols = group_indices(&new, |item| item.symbol_id.clone());
    for symbol in old_symbols
        .keys()
        .filter(|symbol| new_symbols.contains_key(*symbol))
    {
        let left = &old_symbols[symbol];
        let right = &new_symbols[symbol];
        if left.len() == 1 && right.len() == 1 {
            let old_index = left[0];
            let new_index = right[0];
            old_unmatched.remove(&old_index);
            new_unmatched.remove(&new_index);
            let lifecycle = if old[old_index].source.file == new[new_index].source.file {
                IdentityLifecycle::Same
            } else {
                IdentityLifecycle::Moved
            };
            decisions.push(one_to_one(
                lifecycle,
                IdentityConfidence::Exact,
                old[old_index].clone(),
                new[new_index].clone(),
            ));
        } else {
            mark_ambiguous(
                left,
                right,
                &old,
                &new,
                &mut old_unmatched,
                &mut new_unmatched,
                &mut decisions,
            );
        }
    }

    let old_strong = group_unmatched(&old, &old_unmatched, strong_key);
    let new_strong = group_unmatched(&new, &new_unmatched, strong_key);
    for key in old_strong
        .keys()
        .filter(|key| !key.is_empty() && new_strong.contains_key(*key))
    {
        let left = &old_strong[key];
        let right = &new_strong[key];
        if left.len() == 1 && right.len() == 1 {
            let old_index = left[0];
            let new_index = right[0];
            if let Some(lifecycle) =
                conservative_structural_lifecycle(&old[old_index], &new[new_index])
            {
                old_unmatched.remove(&old_index);
                new_unmatched.remove(&new_index);
                decisions.push(one_to_one(
                    lifecycle,
                    IdentityConfidence::Strong,
                    old[old_index].clone(),
                    new[new_index].clone(),
                ));
            } else {
                mark_ambiguous(
                    left,
                    right,
                    &old,
                    &new,
                    &mut old_unmatched,
                    &mut new_unmatched,
                    &mut decisions,
                );
            }
        } else {
            mark_ambiguous(
                left,
                right,
                &old,
                &new,
                &mut old_unmatched,
                &mut new_unmatched,
                &mut decisions,
            );
        }
    }

    for index in old_unmatched {
        decisions.push(IdentityDecision {
            lifecycle: IdentityLifecycle::Deleted,
            confidence: IdentityConfidence::None,
            before: vec![old[index].clone()],
            after: vec![],
            candidates: vec![],
            fact_delta: None,
        });
    }
    let mut introduced: Vec<_> = new_unmatched
        .into_iter()
        .map(|index| new[index].clone())
        .collect();
    decisions.sort_by_key(decision_key);
    introduced.sort_by_key(identity_key);
    Ok(IdentityReport {
        schema: IDENTITY_REPORT_SCHEMA.into(),
        build_model_delta: BuildModelFactDelta {
            project_model_hash: FieldDelta::between(
                Some(before.project_model_hash.clone()),
                Some(after.project_model_hash.clone()),
            ),
            classpath_hash: FieldDelta::between(
                Some(before.classpath_hash.clone()),
                Some(after.classpath_hash.clone()),
            ),
            compiler_options_hash: FieldDelta::between(
                Some(before.compiler_options_hash.clone()),
                Some(after.compiler_options_hash.clone()),
            ),
        },
        before,
        after,
        decisions,
        introduced,
        supported_contour: vec![
            IdentityLifecycle::Same,
            IdentityLifecycle::Renamed,
            IdentityLifecycle::Moved,
            IdentityLifecycle::Deleted,
            IdentityLifecycle::Ambiguous,
        ],
    })
}

fn conservative_structural_lifecycle(
    before: &DeclarationIdentity,
    after: &DeclarationIdentity,
) -> Option<IdentityLifecycle> {
    if before.name == after.name {
        let same_file_name =
            Path::new(&before.source.file).file_name() == Path::new(&after.source.file).file_name();
        return (same_file_name
            && !before.source_signature_hash.is_empty()
            && before.source_signature_hash == after.source_signature_hash)
            .then_some(IdentityLifecycle::Moved);
    }
    let nearby = before.source.range_start.abs_diff(after.source.range_start) <= 256;
    (before.source.file == after.source.file
        && before.containing_declaration == after.containing_declaration
        && nearby)
        .then_some(IdentityLifecycle::Renamed)
}

fn validate_snapshot(snapshot: &SnapshotProvenance) -> Result<(), IdentityError> {
    for (name, value) in [
        ("compositeSnapshotHash", &snapshot.composite_snapshot_hash),
        ("indexSnapshotHash", &snapshot.index_snapshot_hash),
        ("projectModelHash", &snapshot.project_model_hash),
        ("classpathHash", &snapshot.classpath_hash),
        ("compilerOptionsHash", &snapshot.compiler_options_hash),
    ] {
        if value.is_empty() {
            return Err(IdentityError::EmptySnapshotField(name));
        }
    }
    Ok(())
}

fn declarations(files: &[Value]) -> Result<Vec<DeclarationIdentity>, IdentityError> {
    let mut result = Vec::new();
    for file in files {
        let content_hash = text(file, "contentHash");
        for declaration in file
            .get("declarations")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let symbol_id = text(declaration, "symbolId");
            if symbol_id.is_empty() {
                continue;
            }
            let identity = declaration.get("symbolIdentity");
            let identity_shape_hash = identity
                .filter(|identity| complete_identity_shape(identity))
                .map(stable_identity_shape)
                .map(|shape| {
                    canonical::hash(&shape)
                        .map_err(|error| IdentityError::Canonical(error.to_string()))
                })
                .transpose()?
                .unwrap_or_default();
            result.push(DeclarationIdentity {
                declaration_id: text(declaration, "declarationId"),
                symbol_id,
                name: text(declaration, "name"),
                kind: text(declaration, "kind"),
                containing_declaration: declaration
                    .get("containingDeclaration")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
                source: SourceFactProvenance {
                    file: declaration
                        .pointer("/sourceOrigin/file")
                        .and_then(Value::as_str)
                        .filter(|file| !file.is_empty())
                        .unwrap_or_default()
                        .to_owned(),
                    content_hash: content_hash.clone(),
                    range_start: declaration
                        .pointer("/sourceOrigin/rangeStart")
                        .or_else(|| declaration.get("rangeStart"))
                        .and_then(Value::as_u64)
                        .unwrap_or(0),
                    range_end: declaration
                        .pointer("/sourceOrigin/rangeEnd")
                        .or_else(|| declaration.get("rangeEnd"))
                        .and_then(Value::as_u64)
                        .unwrap_or(0),
                },
                source_signature_hash: text(declaration, "sourceSignatureHash"),
                body_hash: text(declaration, "bodyHash"),
                abi_hash: text(declaration, "abiHash"),
                semantic_summary_hash: text(declaration, "semanticSummaryHash"),
                identity_shape_hash,
            });
        }
    }
    result.sort_by_key(identity_key);
    Ok(result)
}

fn stable_identity_shape(identity: &Value) -> Value {
    json!({
        "module":identity.get("module"),
        "sourceSet":identity.get("sourceSet"),
        "declarationKind":identity.get("declarationKind"),
        "typeParameterArity":identity.get("typeParameterArity"),
        "receiverTypes":identity.get("receiverTypes"),
        "contextReceiverTypes":identity.get("contextReceiverTypes"),
        "parameterTypes":identity.get("parameterTypes"),
        "returnType":identity.get("returnType"),
        "suspendFlag":identity.get("suspendFlag"),
    })
}

fn complete_identity_shape(identity: &Value) -> bool {
    let Some(object) = identity.as_object() else {
        return false;
    };
    ["module", "sourceSet", "declarationKind", "returnType"]
        .iter()
        .all(|field| {
            object
                .get(*field)
                .and_then(Value::as_str)
                .is_some_and(|value| !value.is_empty())
        })
        && object
            .get("typeParameterArity")
            .and_then(Value::as_u64)
            .is_some()
        && ["receiverTypes", "contextReceiverTypes", "parameterTypes"]
            .iter()
            .all(|field| {
                object
                    .get(*field)
                    .and_then(Value::as_array)
                    .is_some_and(|values| {
                        values.iter().all(|value| {
                            value
                                .as_str()
                                .is_some_and(|type_name| !type_name.is_empty())
                        })
                    })
            })
        && object.get("suspendFlag").and_then(Value::as_bool).is_some()
}

fn text(value: &Value, field: &str) -> String {
    value
        .get(field)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

fn strong_key(identity: &DeclarationIdentity) -> String {
    if identity.kind.is_empty()
        || identity.declaration_id.is_empty()
        || identity.identity_shape_hash.is_empty()
        || identity.source_signature_hash.is_empty()
        || identity.body_hash.is_empty()
        || identity.abi_hash.is_empty()
        || identity.semantic_summary_hash.is_empty()
        || identity.source.file.is_empty()
        || identity.source.content_hash.is_empty()
        || identity.source.range_end <= identity.source.range_start
    {
        String::new()
    } else {
        canonical::hash(&json!({
            "kind": identity.kind,
            "bodyHash": identity.body_hash,
            "summaryHash": identity.semantic_summary_hash,
            "shapeHash": identity.identity_shape_hash,
        }))
        .unwrap_or_default()
    }
}

fn group_indices<F>(values: &[DeclarationIdentity], key: F) -> BTreeMap<String, Vec<usize>>
where
    F: Fn(&DeclarationIdentity) -> String,
{
    let mut groups = BTreeMap::new();
    for (index, value) in values.iter().enumerate() {
        groups
            .entry(key(value))
            .or_insert_with(Vec::new)
            .push(index);
    }
    groups
}

fn group_unmatched<F>(
    values: &[DeclarationIdentity],
    indices: &BTreeSet<usize>,
    key: F,
) -> BTreeMap<String, Vec<usize>>
where
    F: Fn(&DeclarationIdentity) -> String,
{
    let mut groups = BTreeMap::new();
    for index in indices {
        groups
            .entry(key(&values[*index]))
            .or_insert_with(Vec::new)
            .push(*index);
    }
    groups
}

fn one_to_one(
    lifecycle: IdentityLifecycle,
    confidence: IdentityConfidence,
    before: DeclarationIdentity,
    after: DeclarationIdentity,
) -> IdentityDecision {
    IdentityDecision {
        lifecycle,
        confidence,
        candidates: vec![after.symbol_id.clone()],
        fact_delta: Some(fact_delta(&before, &after)),
        before: vec![before],
        after: vec![after],
    }
}

fn mark_ambiguous(
    left: &[usize],
    right: &[usize],
    old: &[DeclarationIdentity],
    new: &[DeclarationIdentity],
    old_unmatched: &mut BTreeSet<usize>,
    new_unmatched: &mut BTreeSet<usize>,
    decisions: &mut Vec<IdentityDecision>,
) {
    for index in left {
        old_unmatched.remove(index);
    }
    for index in right {
        new_unmatched.remove(index);
    }
    let mut before: Vec<_> = left.iter().map(|index| old[*index].clone()).collect();
    let mut after: Vec<_> = right.iter().map(|index| new[*index].clone()).collect();
    before.sort_by_key(identity_key);
    after.sort_by_key(identity_key);
    decisions.push(IdentityDecision {
        lifecycle: IdentityLifecycle::Ambiguous,
        confidence: IdentityConfidence::Ambiguous,
        candidates: after.iter().map(|item| item.symbol_id.clone()).collect(),
        before,
        after,
        fact_delta: None,
    });
}

fn fact_delta(before: &DeclarationIdentity, after: &DeclarationIdentity) -> DeclarationFactDelta {
    DeclarationFactDelta {
        declaration_id: FieldDelta::between(
            Some(before.declaration_id.clone()),
            Some(after.declaration_id.clone()),
        ),
        symbol_id: FieldDelta::between(
            Some(before.symbol_id.clone()),
            Some(after.symbol_id.clone()),
        ),
        kind: FieldDelta::between(Some(before.kind.clone()), Some(after.kind.clone())),
        containing_declaration: FieldDelta::between(
            before.containing_declaration.clone(),
            after.containing_declaration.clone(),
        ),
        source_origin: FieldDelta::between(
            Some(format!(
                "{}:{}-{}@{}",
                before.source.file,
                before.source.range_start,
                before.source.range_end,
                before.source.content_hash
            )),
            Some(format!(
                "{}:{}-{}@{}",
                after.source.file,
                after.source.range_start,
                after.source.range_end,
                after.source.content_hash
            )),
        ),
        source_signature_hash: FieldDelta::between(
            Some(before.source_signature_hash.clone()),
            Some(after.source_signature_hash.clone()),
        ),
        body_hash: FieldDelta::between(
            Some(before.body_hash.clone()),
            Some(after.body_hash.clone()),
        ),
        abi_hash: FieldDelta::between(Some(before.abi_hash.clone()), Some(after.abi_hash.clone())),
        semantic_summary_hash: FieldDelta::between(
            Some(before.semantic_summary_hash.clone()),
            Some(after.semantic_summary_hash.clone()),
        ),
        identity_shape_hash: FieldDelta::between(
            Some(before.identity_shape_hash.clone()),
            Some(after.identity_shape_hash.clone()),
        ),
    }
}

fn identity_key(value: &DeclarationIdentity) -> (String, String, u64) {
    (
        value.symbol_id.clone(),
        value.source.file.clone(),
        value.source.range_start,
    )
}

fn decision_key(value: &IdentityDecision) -> (String, String) {
    let first = value
        .before
        .first()
        .map(|item| item.symbol_id.clone())
        .unwrap_or_default();
    (first, format!("{:?}", value.lifecycle))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn provenance(index: &str) -> SnapshotProvenance {
        SnapshotProvenance {
            composite_snapshot_hash: format!("composite:{index}"),
            index_snapshot_hash: index.into(),
            project_model_hash: "model".into(),
            classpath_hash: "classpath".into(),
            compiler_options_hash: "options".into(),
        }
    }

    fn file(path: &str, declarations: Vec<Value>) -> Value {
        json!({"path":path,"contentHash":format!("hash:{path}"),"declarations":declarations})
    }

    fn declaration(symbol: &str, name: &str, body: &str, summary: &str) -> Value {
        json!({
            "declarationId":format!("decl:{symbol}"),"symbolId":symbol,"name":name,"kind":"FUNCTION",
            "symbolIdentity":{
                "module":":","sourceSet":"main","package":"p","declarationName":name,
                "containingDeclarations":[],"declarationKind":"FUNCTION","typeParameterArity":0,
                "receiverTypes":[],"contextReceiverTypes":[],"parameterTypes":["String"],
                "returnType":"String","suspendFlag":false,"jvmDescriptor":"(Ljava/lang/String;)Ljava/lang/String;"
            },
            "sourceOrigin":{"file":"A.kt","rangeStart":1,"rangeEnd":20},
            "sourceSignatureHash":format!("sig:{name}"),"bodyHash":body,"abiHash":format!("abi:{symbol}"),"semanticSummaryHash":summary
        })
    }

    #[test]
    fn exact_symbol_is_same_and_file_relocation_is_moved() {
        let before = vec![file(
            "A.kt",
            vec![declaration("p.f", "f", "body", "summary")],
        )];
        let mut moved = declaration("p.f", "f", "body", "summary");
        moved["sourceOrigin"]["file"] = json!("B.kt");
        let after = vec![file("B.kt", vec![moved])];
        let report =
            decide_identity_delta(provenance("old"), provenance("new"), &before, &after).unwrap();
        assert_eq!(report.decisions[0].lifecycle, IdentityLifecycle::Moved);
        assert_eq!(report.decisions[0].confidence, IdentityConfidence::Exact);
    }

    #[test]
    fn unique_structural_match_is_rename() {
        let before = vec![file(
            "A.kt",
            vec![declaration("p.old", "old", "body", "summary")],
        )];
        let after = vec![file(
            "A.kt",
            vec![declaration("p.new", "new", "body", "summary")],
        )];
        let report =
            decide_identity_delta(provenance("old"), provenance("new"), &before, &after).unwrap();
        assert_eq!(report.decisions[0].lifecycle, IdentityLifecycle::Renamed);
        assert_eq!(report.decisions[0].confidence, IdentityConfidence::Strong);
    }

    #[test]
    fn simultaneous_rename_and_move_is_not_guessed() {
        let before = vec![file(
            "A.kt",
            vec![declaration("p.old", "old", "body", "summary")],
        )];
        let mut changed = declaration("q.new", "new", "body", "summary");
        changed["sourceOrigin"]["file"] = json!("B.kt");
        let after = vec![file("B.kt", vec![changed])];
        let report =
            decide_identity_delta(provenance("old"), provenance("new"), &before, &after).unwrap();
        assert_eq!(report.decisions[0].lifecycle, IdentityLifecycle::Ambiguous);
        assert_eq!(
            report.decisions[0].confidence,
            IdentityConfidence::Ambiguous
        );
    }

    #[test]
    fn decoy_and_split_merge_shapes_are_ambiguous() {
        let before = vec![file(
            "A.kt",
            vec![declaration("p.old", "old", "body", "summary")],
        )];
        let after = vec![file(
            "A.kt",
            vec![
                declaration("p.a", "a", "body", "summary"),
                declaration("p.b", "b", "body", "summary"),
            ],
        )];
        let report =
            decide_identity_delta(provenance("old"), provenance("new"), &before, &after).unwrap();
        assert_eq!(report.decisions[0].lifecycle, IdentityLifecycle::Ambiguous);
        assert_eq!(report.decisions[0].after.len(), 2);
        assert!(report.introduced.is_empty());
    }

    #[test]
    fn deletion_and_introduction_do_not_silently_retarget() {
        let before = vec![file(
            "A.kt",
            vec![declaration("p.old", "old", "body:a", "summary:a")],
        )];
        let after = vec![file(
            "B.kt",
            vec![declaration("p.new", "new", "body:b", "summary:b")],
        )];
        let report =
            decide_identity_delta(provenance("old"), provenance("new"), &before, &after).unwrap();
        assert_eq!(report.decisions[0].lifecycle, IdentityLifecycle::Deleted);
        assert_eq!(report.introduced[0].symbol_id, "p.new");
    }

    #[test]
    fn incomplete_legacy_facts_never_get_structural_continuity() {
        let legacy = |symbol: &str, name: &str| {
            json!({
                "symbolId": symbol,
                "name": name,
                "kind": "FUNCTION",
                "sourceOrigin": {"file": "A.kt", "rangeStart": 1, "rangeEnd": 20},
                "bodyHash": "same-body",
                "semanticSummaryHash": "same-summary"
            })
        };
        let before = vec![file("A.kt", vec![legacy("p.old", "old")])];
        let after = vec![file("A.kt", vec![legacy("p.unrelated", "unrelated")])];

        let report =
            decide_identity_delta(provenance("old"), provenance("new"), &before, &after).unwrap();

        assert_eq!(report.decisions[0].lifecycle, IdentityLifecycle::Deleted);
        assert_eq!(report.decisions[0].confidence, IdentityConfidence::None);
        assert_eq!(report.introduced[0].symbol_id, "p.unrelated");
        assert!(report.decisions.iter().all(|decision| !matches!(
            decision.lifecycle,
            IdentityLifecycle::Renamed | IdentityLifecycle::Moved
        )));
    }

    #[test]
    fn null_identity_shape_never_gets_structural_continuity() {
        let malformed = |symbol: &str, name: &str| {
            let mut value = declaration(symbol, name, "same-body", "same-summary");
            value["symbolIdentity"] = json!({
                "module": null,
                "sourceSet": null,
                "declarationKind": null,
                "typeParameterArity": null,
                "receiverTypes": null,
                "contextReceiverTypes": null,
                "parameterTypes": null,
                "returnType": null,
                "suspendFlag": null
            });
            value
        };
        let before = vec![file("A.kt", vec![malformed("p.old", "old")])];
        let after = vec![file("A.kt", vec![malformed("p.unrelated", "unrelated")])];

        let report =
            decide_identity_delta(provenance("old"), provenance("new"), &before, &after).unwrap();

        assert_eq!(report.decisions[0].lifecycle, IdentityLifecycle::Deleted);
        assert_eq!(report.introduced[0].symbol_id, "p.unrelated");
    }

    #[test]
    fn missing_source_origin_never_gets_structural_continuity() {
        let without_origin = |symbol: &str, name: &str| {
            let mut value = declaration(symbol, name, "same-body", "same-summary");
            value.as_object_mut().unwrap().remove("sourceOrigin");
            value
        };
        let before = vec![file("A.kt", vec![without_origin("p.old", "old")])];
        let after = vec![file(
            "A.kt",
            vec![without_origin("p.unrelated", "unrelated")],
        )];

        let report =
            decide_identity_delta(provenance("old"), provenance("new"), &before, &after).unwrap();

        assert_eq!(report.decisions[0].lifecycle, IdentityLifecycle::Deleted);
        assert_eq!(report.introduced[0].symbol_id, "p.unrelated");
    }

    #[test]
    fn output_is_deterministic_under_input_order() {
        let a = declaration("p.a", "a", "body:a", "summary:a");
        let b = declaration("p.b", "b", "body:b", "summary:b");
        let left = decide_identity_delta(
            provenance("x"),
            provenance("y"),
            &[file("A.kt", vec![a.clone(), b.clone()])],
            &[file("A.kt", vec![a.clone(), b.clone()])],
        )
        .unwrap();
        let right = decide_identity_delta(
            provenance("x"),
            provenance("y"),
            &[file("A.kt", vec![b.clone(), a.clone()])],
            &[file("A.kt", vec![b, a])],
        )
        .unwrap();
        assert_eq!(
            canonical::bytes(&left).unwrap(),
            canonical::bytes(&right).unwrap()
        );
    }
}
