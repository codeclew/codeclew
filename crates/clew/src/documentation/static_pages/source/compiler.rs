//! Producer admission for shared retained structure and exact-call navigation.
//! Portable signature grammar does not transfer Java compiler authority.
use super::super::model::{CallableProjection, ProjectionKind};
use super::{CallableKey, Context, ProjectedCallable, gap};
use crate::documentation::{digest, invalid, model::Observation};
use crate::error::ClewError;

pub(super) fn csharp_candidate(owner: &Observation) -> bool {
    owner.normalized["schema"]
        .as_str()
        .is_some_and(|schema| schema.starts_with("codeclew-csharp-compiler-fact/"))
}

pub(super) fn csharp_admitted(owner: &Observation) -> bool {
    let n = &owner.normalized;
    n["schema"] == crate::csharp_adapter_v2::CSHARP_FACT_SCHEMA
        && owner.kind == "SYMBOL"
        && n["kind"] == "DECLARATION"
        && n["declarationKind"] == "METHOD"
        && n["resolution"] == "COMPILER_EXACT"
        && n["symbolIdentity"] == owner.symbol
        && n["scope"].as_str().is_some_and(|s| !s.is_empty())
        && n["csharpIdentity"]
            .as_str()
            .is_some_and(|s| s.starts_with("M:"))
        && n["name"]
            .as_str()
            .zip(n["jvmDescriptor"].as_str())
            .is_some_and(|(name, descriptor)| {
                n["ownerIdentity"].as_str().is_some_and(|class| {
                    class.starts_with("class:")
                        && (owner.symbol == format!("method:{class}#{name}{descriptor}")
                            || n["csharpIdentity"].as_str().is_some_and(|id| {
                                owner.symbol
                                    == format!(
                                        "method:{class}#{name}{descriptor}@{}",
                                        &crate::canonical::hash_bytes(id.as_bytes())[7..19]
                                    )
                            }))
                })
            })
}

pub(in crate::documentation::static_pages) fn admitted(owner: &Observation) -> bool {
    let n = &owner.normalized;
    csharp_admitted(owner)
        || (owner.kind == "SYMBOL"
            && n["schema"] == "declaration-descriptor/0.1"
            && n["declarationKind"] == "FUNCTION"
            && n["resolution"] == "PROVEN"
            && n["provider"] == "K2_FIR"
            && n["sourceProvenance"] == "COMPILER_UTF16_RANGE_TO_UTF8_BYTES"
            && n["compilerAuthority"] == "fir-facts-extractor/0.6"
            && n["symbolIdentity"] == owner.symbol
            && crate::semantic_validation::validate_kotlin_full_symbol_identity(&owner.symbol)
                .is_ok())
}

pub(in crate::documentation::static_pages) fn body_envelope(owner: &Observation) -> bool {
    super::outline::producer(owner).is_some()
        && owner.normalized["documentation"]["events"].is_array()
        && owner.normalized["documentation"]["boundaries"].is_array()
}

pub(super) fn project_csharp(
    context: &mut Context<'_>,
    id: &str,
) -> Result<ProjectedCallable, ClewError> {
    let owner = context
        .evidence
        .observations
        .get(id)
        .ok_or_else(|| invalid("compiler declaration is missing"))?
        .clone();
    if !csharp_admitted(&owner)
        || owner.id != id
        || owner.service != context.evidence.service
        || owner.digest != digest(&owner.normalized)?
    {
        return Err(invalid(
            "Roslyn METHOD declaration lacks exact retained compiler authority",
        ));
    }
    let scope = owner.normalized["scope"].as_str().unwrap();
    if context
        .evidence
        .observations
        .values()
        .filter(|candidate| {
            candidate.kind == "SYMBOL"
                && candidate.symbol == owner.symbol
                && candidate.normalized["scope"] == scope
        })
        .count()
        != 1
    {
        return Err(invalid(
            "Roslyn METHOD identity is ambiguous within its compilation scope",
        ));
    }
    let source = match owner.source_ids.as_slice() {
        [id] => context
            .evidence
            .sources
            .get(id)
            .filter(|source| {
                source.id == *id
                    && context.valid_source(source)
                    && crate::documentation::store::relative(&source.file).is_ok()
            })
            .cloned(),
        _ => None,
    }
    .ok_or_else(|| {
        invalid("Roslyn METHOD requires one valid retained compiler-bound source span")
    })?;
    let citation = context.citation(&source, 0, source.text.len());
    context.retain(&owner);
    let mut projection = CallableProjection {
        declaration_id: owner.id.clone(),
        symbol: owner.symbol.clone(),
        authority: "COMPILER_DECLARATION".into(),
        citation_id: Some(citation.clone()),
        control_flow: None,
        source_outline: None,
        retained_call_sites: None,
        steps: vec![],
        state: vec![],
        gaps: vec![
            gap(
                "SOURCE_BEHAVIOR_PROJECTION_UNAVAILABLE",
                "Retained source structure and compiler call targets do not establish statement effects or runtime behavior.",
                Some(citation.clone()),
            ),
            gap(
                "SOURCE_CALL_GRAPH_UNAVAILABLE",
                "Source-call body expansion is unavailable for this declaration page.",
                Some(citation.clone()),
            ),
            gap(
                "DATA_STATE_UNAVAILABLE",
                "Data-flow and state transformations are unavailable for this declaration page.",
                Some(citation),
            ),
        ],
    };
    super::outline::attach(context, &owner, scope, &mut projection)?;
    super::call_sites::attach(context, &owner, scope, &mut projection)?;
    Ok(ProjectedCallable {
        key: CallableKey {
            service: owner.service,
            scope: scope.into(),
            symbol: owner.symbol,
        },
        projection,
        kind: ProjectionKind::DeclarationOnly,
    })
}
