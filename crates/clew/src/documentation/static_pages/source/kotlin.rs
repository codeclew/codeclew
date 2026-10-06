//! Strict compiler-admitted Kotlin declaration projection.
use super::super::model::{CallableProjection, ProjectionKind};
use super::{CallableKey, Context, ProjectedCallable, gap};
use crate::documentation::{digest, invalid, model::Observation};
use crate::error::ClewError;

const KOTLIN_SCHEMA: &str = "declaration-descriptor/0.1";

pub(super) fn candidate(observation: &Observation) -> bool {
    observation.normalized["schema"] == KOTLIN_SCHEMA
        || observation.normalized["schema"]
            .as_str()
            .is_some_and(|schema| schema.starts_with("declaration-descriptor/"))
        || observation.normalized["compilerCallableId"].is_string()
        || observation.symbol.starts_with("callable:")
        || matches!(
            observation.normalized["syntaxKind"].as_str(),
            Some(
                "function_declaration"
                    | "property_declaration"
                    | "class_declaration"
                    | "secondary_constructor"
            )
        )
}

pub(super) fn project(context: &mut Context<'_>, id: &str) -> Result<ProjectedCallable, ClewError> {
    let observation = context
        .evidence
        .observations
        .get(id)
        .ok_or_else(|| invalid(format!("selected Kotlin declaration {id} is missing")))?
        .clone();
    let normalized = &observation.normalized;
    if normalized["schema"] != KOTLIN_SCHEMA {
        return Err(invalid(
            "Kotlin pages require compiler-admitted declaration-descriptor/0.1 facts; syntax-only declarations are unsupported",
        ));
    }
    if observation.kind != "SYMBOL" || observation.service != context.evidence.service {
        return Err(invalid(
            "Kotlin declaration service or observation identity does not match the selected Check service",
        ));
    }
    if observation.id != id {
        return Err(invalid(
            "Kotlin declaration observation ID does not match the selected Check key",
        ));
    }
    if normalized["declarationKind"] != "FUNCTION" {
        return Err(invalid(
            "Kotlin native pages currently support compiler FUNCTION declarations only",
        ));
    }
    if normalized["resolution"] != "PROVEN"
        || normalized["provider"] != "K2_FIR"
        || normalized["sourceProvenance"] != "COMPILER_UTF16_RANGE_TO_UTF8_BYTES"
        || normalized["compilerAuthority"] != "fir-facts-extractor/0.6"
    {
        return Err(invalid(
            "Kotlin FUNCTION declaration lacks exact compiler authority",
        ));
    }
    if normalized["symbolIdentity"] != observation.symbol {
        return Err(invalid(
            "Kotlin compiler symbol identity does not match the selected observation",
        ));
    }
    let callable_id = normalized["compilerCallableId"]
        .as_str()
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| invalid("Kotlin FUNCTION compiler callable identity is missing"))?;
    let symbol_prefix = format!("callable:{callable_id}#jvm:");
    let jvm_signature = observation
        .symbol
        .strip_prefix(&symbol_prefix)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            invalid("Kotlin compiler callable identity disagrees with the selected symbol")
        })?;
    crate::semantic_validation::validate_kotlin_full_symbol_identity(&observation.symbol)
        .map_err(|_| invalid("Kotlin FUNCTION full symbol identity is invalid"))?;
    if let Some(value) = normalized.get("jvmDescriptor") {
        let descriptor = value
            .as_str()
            .ok_or_else(|| invalid("Kotlin JVM descriptor is not a string"))?;
        if descriptor.is_empty() || descriptor != jvm_signature {
            return Err(invalid(
                "Kotlin JVM descriptor disagrees with the compiler symbol identity",
            ));
        }
    }
    let module = normalized["module"]
        .as_str()
        .filter(|value| !value.is_empty())
        .ok_or_else(|| invalid("Kotlin FUNCTION module identity is missing"))?;
    let source_set = normalized["sourceSet"]
        .as_str()
        .filter(|value| !value.is_empty())
        .ok_or_else(|| invalid("Kotlin FUNCTION source-set identity is missing"))?;
    let scope = normalized["scope"]
        .as_str()
        .filter(|value| !value.is_empty())
        .ok_or_else(|| invalid("Kotlin FUNCTION compilation scope is missing"))?;
    if scope != format!("{module}/{source_set}") {
        return Err(invalid(
            "Kotlin FUNCTION scope does not match its compiler module and source set",
        ));
    }
    if digest(normalized)? != observation.digest {
        return Err(invalid(
            "Kotlin FUNCTION normalized evidence digest is inconsistent",
        ));
    }
    let candidates: Vec<_> = context
        .evidence
        .observations
        .values()
        .filter(|candidate| {
            candidate.kind == "SYMBOL"
                && candidate.service == context.evidence.service
                && candidate.normalized["schema"] == KOTLIN_SCHEMA
                && candidate.normalized["symbolIdentity"] == observation.symbol
                && candidate.normalized["scope"] == scope
        })
        .collect();
    if candidates.len() != 1 || candidates[0].id != observation.id {
        return Err(invalid(
            "Kotlin FUNCTION identity is ambiguous within its compilation scope",
        ));
    }
    let source = match observation.source_ids.as_slice() {
        [source_id] => context
            .evidence
            .sources
            .get(source_id)
            .filter(|source| source.id == *source_id)
            .filter(|source| context.valid_source(source))
            .filter(|source| crate::documentation::store::relative(&source.file).is_ok())
            .cloned(),
        _ => None,
    }
    .ok_or_else(|| {
        invalid("Kotlin FUNCTION requires one valid retained compiler-bound source span")
    })?;

    // The citation is relative to retained text. A missing SourceOccurrence
    // remains a line-span citation and is never promoted to an exact body span.
    let citation_id = context.citation(&source, 0, source.text.len());
    context.retain(&observation);
    let mut projection = CallableProjection {
        declaration_id: observation.id.clone(),
        symbol: observation.symbol.clone(),
        authority: "COMPILER_DECLARATION".into(),
        citation_id: Some(citation_id.clone()),
        control_flow: None,
        steps: vec![],
        state: vec![],
        gaps: vec![
            gap(
                "KOTLIN_BEHAVIOR_PROJECTION_UNAVAILABLE",
                "Kotlin statement-level behavior is unavailable; any admitted compiler control-flow panel is shown separately.",
                Some(citation_id.clone()),
            ),
            gap(
                "KOTLIN_SOURCE_CALL_GRAPH_UNAVAILABLE",
                "Kotlin source-call graph expansion is unavailable for this declaration page.",
                Some(citation_id.clone()),
            ),
            gap(
                "KOTLIN_DATA_STATE_UNAVAILABLE",
                "Kotlin data-flow and state transformations are unavailable for this declaration page.",
                Some(citation_id),
            ),
        ],
    };
    super::control_flow::attach(context, &observation, scope, &mut projection)?;
    let kind = if projection.control_flow.is_some() {
        ProjectionKind::CompilerControlFlow
    } else {
        ProjectionKind::DeclarationOnly
    };
    Ok(ProjectedCallable {
        key: CallableKey {
            service: context.evidence.service.clone(),
            scope: scope.to_owned(),
            symbol: observation.symbol,
        },
        projection,
        kind,
    })
}
