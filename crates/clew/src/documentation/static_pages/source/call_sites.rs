//! Neutral exact call-site facts admitted from retained Kotlin compiler relations.
use super::super::model::{
    CallableProjection, Citation, NeutralCallArgumentBinding, NeutralCallArgumentBindings,
    NeutralExactCallSite, NeutralExactCallSiteOwnerKey, RetainedCallSites,
};
use super::{Context, gap};
use crate::documentation::model::{Observation, ServiceEvidence, Source};
use crate::documentation::{digest, invalid};
use crate::error::ClewError;
use std::collections::{BTreeMap, BTreeSet};

const CALL_SCHEMA: &str = "codeclew-kotlin-documentation-call/1.0";
const NO_CALL_SITES: &str = "KOTLIN_RETAINED_CALL_SITES_NOT_PROVEN";
const REJECTED_CALL_SITES: &str = "KOTLIN_RETAINED_CALL_SITES_REJECTED";
const CONFLICTING_CALL_SITES: &str = "KOTLIN_RETAINED_CALL_SITE_CONFLICT";
const ARGUMENT_BINDINGS_SCHEMA: &str = "codeclew-call-argument-bindings/1.0";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UnavailableReason {
    NotProven,
    Rejected,
    Conflict,
}

impl UnavailableReason {
    fn code(self) -> &'static str {
        match self {
            Self::NotProven => NO_CALL_SITES,
            Self::Rejected => REJECTED_CALL_SITES,
            Self::Conflict => CONFLICTING_CALL_SITES,
        }
    }

    fn detail(self) -> &'static str {
        match self {
            Self::NotProven => {
                "No exact retained Kotlin call-site evidence is available; absence does not establish that this declaration has no calls."
            }
            Self::Rejected => {
                "Retained Kotlin call-site evidence did not satisfy the exact owner, target, source and binding contract; no sites are projected."
            }
            Self::Conflict => {
                "Multiple retained compiler relations bind the same captured source occurrence; no sites are projected for the conflicting set."
            }
        }
    }
}

/// Attach the selected function's exact call-site evidence independently of
/// statement parsing, source outlines, CFG, or expansion decisions.
pub(super) fn attach(
    context: &mut Context<'_>,
    owner: &Observation,
    scope: &str,
    callable: &mut CallableProjection,
) -> Result<(), ClewError> {
    let owner_key = NeutralExactCallSiteOwnerKey {
        service: context.evidence.service.clone(),
        scope: scope.to_owned(),
        symbol: owner.symbol.clone(),
    };
    let candidates = candidate_relations(&context.evidence.observations, &owner.symbol, scope);
    for (_, relation) in &candidates {
        context.retain(relation);
    }
    let owner_source = owner_source(context.evidence, owner);
    let projection = derive_projection(
        &context.evidence.service,
        &context.evidence.revision,
        &owner_key,
        owner,
        owner_source,
        &candidates,
        &context.evidence.sources,
        callable.citation_id.clone(),
    )?;
    let projection = augment_source_events(
        projection,
        owner,
        owner_source,
        &context.evidence.observations,
        &context.evidence.sources,
    )?;
    for site in &projection.sites {
        if let Some(observation) = context.evidence.observations.get(&site.relation_id) {
            context.retain(observation);
        }
    }
    if projection.gaps.is_empty() {
        for site in &projection.sites {
            let source = context
                .evidence
                .sources
                .get(&site.source_id)
                .ok_or_else(|| invalid("Kotlin call-site source disappeared after admission"))?;
            let citation_id = context.citation(source, 0, source.text.len());
            if citation_id != site.citation_id {
                return Err(invalid(
                    "Kotlin call-site citation differs from its validated retained span",
                ));
            }
            if let Some(bindings) = &site.argument_bindings
                && bindings.gaps.is_empty()
            {
                for argument in &bindings.arguments {
                    let (start, end) = argument_source_range(site, argument).ok_or_else(|| {
                        invalid("Kotlin argument source range is invalid after admission")
                    })?;
                    if context.citation(source, start, end) != argument.citation_id {
                        return Err(invalid(
                            "Kotlin argument citation differs from its validated retained subspan",
                        ));
                    }
                }
            }
        }
    }
    callable.retained_call_sites = Some(projection);
    Ok(())
}

fn candidate_relations<'a>(
    observations: &'a BTreeMap<String, Observation>,
    owner_symbol: &str,
    scope: &str,
) -> Vec<(&'a str, &'a Observation)> {
    observations
        .iter()
        .filter(|(_, relation)| {
            relation.kind == "CALL_RELATION"
                && relation.normalized["relationKind"] == "CALLS"
                && (relation.symbol == owner_symbol
                    || relation.normalized["sourceIdentity"].as_str() == Some(owner_symbol))
                && relation.normalized["scope"]
                    .as_str()
                    .is_none_or(|relation_scope| relation_scope == scope)
        })
        .map(|(key, relation)| (key.as_str(), relation))
        .collect()
}

fn owner_source<'a>(evidence: &'a ServiceEvidence, owner: &Observation) -> Option<&'a Source> {
    match owner.source_ids.as_slice() {
        [id] => evidence.sources.get(id).filter(|source| source.id == *id),
        _ => None,
    }
}

// Keep independent owner, source and provenance pins explicit at this validation boundary.
#[allow(clippy::too_many_arguments)]
fn derive_projection(
    service: &str,
    revision: &str,
    owner_key: &NeutralExactCallSiteOwnerKey,
    owner: &Observation,
    owner_source: Option<&Source>,
    relations: &[(&str, &Observation)],
    sources: &BTreeMap<String, Source>,
    owner_citation: Option<String>,
) -> Result<RetainedCallSites, ClewError> {
    let unavailable = |reason: UnavailableReason| RetainedCallSites {
        owner_key: owner_key.clone(),
        sites: Vec::new(),
        gaps: vec![gap(
            &if super::compiler::csharp_admitted(owner) {
                reason.code().replace("KOTLIN_", "")
            } else {
                reason.code().into()
            },
            if super::compiler::csharp_admitted(owner) {
                reason.detail().replace("Kotlin", "Roslyn")
            } else {
                reason.detail().into()
            },
            owner_citation.clone(),
        )],
    };

    if owner.id.is_empty()
        || owner.kind != "SYMBOL"
        || owner.service != service
        || owner.symbol != owner_key.symbol
        || owner.normalized["scope"].as_str() != Some(owner_key.scope.as_str())
        || !super::compiler::admitted(owner)
        || owner.digest != digest(&owner.normalized)?
    {
        return Ok(unavailable(UnavailableReason::Rejected));
    }
    let Some(owner_source) = owner_source.filter(|source| {
        valid_source(service, revision, source)
            && source.service == owner_key.service
            && crate::documentation::store::relative(&source.file).is_ok()
    }) else {
        return Ok(unavailable(UnavailableReason::Rejected));
    };
    if relations.is_empty() {
        return Ok(unavailable(UnavailableReason::NotProven));
    }
    let mut occurrences = BTreeSet::new();
    for (_, relation) in relations {
        if let Some(key) = captured_occurrence_key(relation)
            && !occurrences.insert(key)
        {
            return Ok(unavailable(UnavailableReason::Conflict));
        }
    }
    let mut sites = Vec::with_capacity(relations.len());
    for (map_key, relation) in relations {
        let Some((site, argument_gaps)) = relation_site(
            service,
            revision,
            map_key,
            relation,
            owner_key,
            owner,
            owner_source,
            sources,
        )?
        else {
            return Ok(unavailable(UnavailableReason::Rejected));
        };
        if !argument_gaps.is_empty() {
            // Argument mappings are a child of the exact site. An unsupported
            // or malformed child never invalidates the compiler-exact call.
            let mut site = site;
            if let Some(bindings) = &mut site.argument_bindings {
                bindings.gaps = argument_gaps;
            }
            sites.push(site);
            continue;
        }
        sites.push(site);
    }
    sites.sort_by(|left, right| {
        left.file
            .cmp(&right.file)
            .then_with(|| left.start_line.cmp(&right.start_line))
            .then_with(|| {
                left.compilation_byte_start
                    .cmp(&right.compilation_byte_start)
            })
            .then_with(|| left.compilation_byte_end.cmp(&right.compilation_byte_end))
            .then_with(|| left.relation_id.cmp(&right.relation_id))
    });
    Ok(RetainedCallSites {
        owner_key: owner_key.clone(),
        sites,
        gaps: Vec::new(),
    })
}

fn captured_occurrence_key(relation: &Observation) -> Option<(String, String, u64, u64)> {
    let site = &relation.normalized["callSite"];
    Some((
        site["file"].as_str()?.to_owned(),
        site["fullCompilationSourceDigest"].as_str()?.to_owned(),
        site["byteStart"].as_u64()?,
        site["byteEnd"].as_u64()?,
    ))
}

/// The compiler-linked structure envelope retains selected targets separately
/// from mutation relations. Admit that evidence without granting receiver,
/// argument mapping, branch truth or data-flow authority. Older events without
/// the exact shared source-span contract never enter this navigation path.
fn augment_source_events(
    mut retained: RetainedCallSites,
    owner: &Observation,
    owner_source: Option<&Source>,
    observations: &BTreeMap<String, Observation>,
    sources: &BTreeMap<String, Source>,
) -> Result<RetainedCallSites, ClewError> {
    use crate::documentation::source_statement::{SourceStatement, StructureProducer};
    let Some(capabilities) = super::compiler::capabilities(owner) else {
        return Ok(retained);
    };
    if capabilities.source_structure.is_none()
        || (!retained.gaps.is_empty()
            && !retained
                .gaps
                .iter()
                .all(|gap| gap.code.ends_with("RETAINED_CALL_SITES_NOT_PROVEN")))
    {
        return Ok(retained);
    }
    let Some(owner_source) = owner_source else {
        return Ok(retained);
    };
    let scope = retained.owner_key.scope.as_str();
    let Some(events) = owner.normalized["documentation"]["events"].as_array() else {
        return Ok(retained);
    };
    let flows: Vec<_> = observations
        .iter()
        .filter(|(_, flow)| {
            flow.kind == "FLOW"
                && flow.service == owner.service
                && flow.symbol == owner.symbol
                && flow.normalized["scope"] == scope
        })
        .collect();
    if flows.len() != events.len() {
        return Ok(retained);
    }
    let mut ordered = BTreeMap::new();
    for (map_key, flow) in flows {
        let Some(ordinal) = flow.normalized["ordinal"].as_u64() else {
            return Ok(retained);
        };
        if ordered.insert(ordinal, (map_key, flow)).is_some() {
            return Ok(retained);
        }
    }
    for (index, event) in events.iter().enumerate() {
        let Some((map_key, flow)) = ordered.get(&(index as u64)) else {
            return Ok(retained);
        };
        if flow.id != **map_key
            || flow.digest != digest(&flow.normalized)?
            || super::outline::event_payload(&flow.normalized)
                != super::outline::documented_payload(event)
        {
            return Ok(retained);
        }
    }
    for (index, event) in events.iter().enumerate() {
        let SourceStatement::Invocation(call) = SourceStatement::decode(event) else {
            continue;
        };
        if call.kind != crate::documentation::source_statement::InvocationKind::Call {
            continue;
        }
        let Some(target) = call.exact_target() else {
            continue;
        };
        let target_valid = match capabilities.producer {
            StructureProducer::KotlinPsi => {
                crate::semantic_validation::validate_kotlin_full_symbol_identity(target).is_ok()
            }
            StructureProducer::Roslyn => target.starts_with("method:class:"),
            StructureProducer::Javac => false, // Java retains its qualified behavioral relation consumer.
        };
        if !target_valid {
            continue;
        }
        let (_, flow) = ordered[&(index as u64)];
        let Some((source, exact)) = crate::documentation::source_span::retained_event_source(
            owner,
            scope,
            index as u64,
            event,
            flow,
            owner_source,
            sources,
        ) else {
            continue;
        };
        let same = retained.sites.iter().find(|site| {
            site.file == source.file
                && site.full_compilation_source_digest == exact.full_compilation_source_digest
                && site.compilation_byte_start == exact.compilation_byte_start as u64
                && site.compilation_byte_end == exact.compilation_byte_end as u64
        });
        if let Some(same) = same {
            if same.target_identity != target {
                retained.sites.clear();
                retained.gaps = vec![gap(
                    "SOURCE_INVOCATION_TARGET_CONFLICT",
                    "Retained compiler records disagree on the selected target at one exact source occurrence.",
                    None,
                )];
                return Ok(retained);
            }
            continue;
        }
        retained.sites.push(NeutralExactCallSite {
            relation_id: flow.id.clone(),
            normalized_digest: flow.digest.clone(),
            target_identity: target.into(),
            source_id: source.id.clone(),
            file: source.file.clone(),
            start_line: source.start_line,
            end_line: source.end_line,
            compilation_byte_start: exact.compilation_byte_start as u64,
            compilation_byte_end: exact.compilation_byte_end as u64,
            source_digest: source.text_digest.clone(),
            evidence_binding: source.evidence_digest.clone(),
            full_compilation_source_digest: exact.full_compilation_source_digest,
            expression: exact.expression,
            citation_id: citation_id(source, 0, source.text.len()),
            argument_bindings: None,
        });
    }
    if !retained.sites.is_empty() {
        retained.gaps.clear();
    }
    retained.sites.sort_by(|left, right| {
        left.file
            .cmp(&right.file)
            .then_with(|| left.start_line.cmp(&right.start_line))
            .then_with(|| {
                left.compilation_byte_start
                    .cmp(&right.compilation_byte_start)
            })
            .then_with(|| left.compilation_byte_end.cmp(&right.compilation_byte_end))
            .then_with(|| left.relation_id.cmp(&right.relation_id))
    });
    Ok(retained)
}

pub(in crate::documentation::static_pages) fn uses_source_events(
    callable: &CallableProjection,
    observations: &BTreeMap<String, Observation>,
) -> bool {
    callable.retained_call_sites.as_ref().is_some_and(|sites| {
        sites.sites.iter().any(|site| {
            observations
                .get(&site.relation_id)
                .is_some_and(|observation| observation.kind == "FLOW")
        })
    })
}

// Keep independent owner, source and provenance pins explicit at this validation boundary.
#[allow(clippy::too_many_arguments)]
fn relation_site(
    service: &str,
    revision: &str,
    map_key: &str,
    relation: &Observation,
    owner_key: &NeutralExactCallSiteOwnerKey,
    owner: &Observation,
    owner_source: &Source,
    sources: &BTreeMap<String, Source>,
) -> Result<Option<(NeutralExactCallSite, Vec<super::super::model::Gap>)>, ClewError> {
    let normalized = &relation.normalized;
    let Some(target_identity) = normalized["targetIdentity"].as_str() else {
        return Ok(None);
    };
    let csharp = super::compiler::csharp_admitted(owner);
    let producer_valid = if csharp {
        normalized["schema"] == crate::csharp_adapter_v2::CSHARP_FACT_SCHEMA
            && normalized["resolution"] == "COMPILER_EXACT"
            && normalized["sourceIdentity"] == owner.symbol
            && target_identity.starts_with("method:class:")
            && normalized["targetCsharpIdentity"]
                .as_str()
                .is_some_and(|id| id.starts_with("csharp:M:"))
    } else {
        kotlin_relation_valid(normalized, owner, target_identity)
    };
    let target_descriptor = normalized["targetJvmDescriptor"]
        .as_str()
        .unwrap_or_default();
    let site = &normalized["callSite"];
    let Some(file) = site["file"].as_str() else {
        return Ok(None);
    };
    let Some(source_id) = site["sourceId"].as_str() else {
        return Ok(None);
    };
    let Some(start_line) = site["startLine"].as_u64() else {
        return Ok(None);
    };
    let Some(end_line) = site["endLine"].as_u64() else {
        return Ok(None);
    };
    let Some(byte_start) = site["byteStart"].as_u64() else {
        return Ok(None);
    };
    let Some(byte_end) = site["byteEnd"].as_u64() else {
        return Ok(None);
    };
    let Some(source_digest) = site["sourceDigest"].as_str() else {
        return Ok(None);
    };
    let Some(evidence_binding) = normalized["evidenceBinding"].as_str() else {
        return Ok(None);
    };
    let Some(site_evidence_digest) = site["evidenceDigest"].as_str() else {
        return Ok(None);
    };
    let Some(full_source_digest) = site["fullCompilationSourceDigest"].as_str() else {
        return Ok(None);
    };
    let [relation_source_id] = relation.source_ids.as_slice() else {
        return Ok(None);
    };
    let Some(source) = sources
        .get(relation_source_id)
        .filter(|source| source.id == *relation_source_id)
    else {
        return Ok(None);
    };
    if relation.id.is_empty()
        || relation.id != map_key
        || relation.service != service
        || relation.symbol != owner_key.symbol
        || !producer_valid
        || normalized["kind"] != "RELATION"
        || normalized["relationKind"] != "CALLS"
        || normalized["sourceIdentity"].as_str() != Some(owner_key.symbol.as_str())
        || normalized["scope"].as_str() != Some(owner_key.scope.as_str())
        || relation.digest != digest(&relation.normalized)?
        || site["sourceStatus"] != "SOURCE_RETAINED"
        || source_id != relation_source_id
        || site_evidence_digest != evidence_binding
        || source.evidence_digest != evidence_binding
        || !canonical_sha256(evidence_binding)
        || !canonical_sha256(full_source_digest)
        || site["file"].as_str() != Some(source.file.as_str())
        || source.file != owner_source.file
        || source.service != owner_key.service
        || source.revision != revision
        || source.start_line != start_line
        || source.end_line != end_line
        || source.text.is_empty()
        || byte_start >= byte_end
        || byte_end.checked_sub(byte_start) != Some(source.text.len() as u64)
        || site["sourceDigest"].as_str() != Some(source.text_digest.as_str())
        || !valid_source(service, revision, source)
        || !valid_source(service, revision, owner_source)
        || source.start_line < owner_source.start_line
        || source.end_line > owner_source.end_line
        || crate::documentation::store::relative(file).is_err()
    {
        return Ok(None);
    }

    if csharp {
        let owner_site = &owner.normalized["outlineOwnerSource"];
        let (Some(start), Some(end)) = (
            owner_site["byteStart"].as_u64(),
            owner_site["byteEnd"].as_u64(),
        ) else {
            return Ok(None);
        };
        if owner_site["sourceStatus"] != "SOURCE_RETAINED"
            || owner_site["sourceId"] != owner_source.id
            || owner_site["sourceDigest"] != owner_source.text_digest
            || owner_site["evidenceDigest"] != owner_source.evidence_digest
            || owner_site["fullCompilationSourceDigest"] != full_source_digest
            || byte_start < start
            || byte_end > end
            || end.checked_sub(start) != Some(owner_source.text.len() as u64)
            || owner_source
                .text
                .get((byte_start - start) as usize..(byte_end - start) as usize)
                != Some(source.text.as_str())
        {
            return Ok(None);
        }
    }
    let citation_id = citation_id(source, 0, source.text.len());
    let mut neutral_site = NeutralExactCallSite {
        relation_id: relation.id.clone(),
        normalized_digest: relation.digest.clone(),
        target_identity: target_identity.to_owned(),
        source_id: source.id.clone(),
        file: source.file.clone(),
        start_line,
        end_line,
        compilation_byte_start: byte_start,
        compilation_byte_end: byte_end,
        source_digest: source_digest.to_owned(),
        evidence_binding: evidence_binding.to_owned(),
        full_compilation_source_digest: full_source_digest.to_owned(),
        expression: source.text.clone(),
        citation_id,
        argument_bindings: None,
    };
    let (argument_bindings, argument_gaps) = project_argument_bindings(
        normalized.get("argumentBindings"),
        site,
        target_descriptor,
        source,
        &neutral_site,
    );
    neutral_site.argument_bindings = argument_bindings;
    Ok(Some((neutral_site, argument_gaps)))
}

fn kotlin_relation_valid(
    normalized: &serde_json::Value,
    owner: &Observation,
    target: &str,
) -> bool {
    let (
        Some(source_callable),
        Some(source_descriptor),
        Some(target_callable),
        Some(target_descriptor),
    ) = (
        normalized["sourceCompilerCallableId"].as_str(),
        normalized["sourceJvmDescriptor"].as_str(),
        normalized["targetCompilerCallableId"].as_str(),
        normalized["targetJvmDescriptor"].as_str(),
    )
    else {
        return false;
    };
    let source = format!("callable:{source_callable}#jvm:{source_descriptor}");
    normalized["schema"] == CALL_SCHEMA
        && normalized["resolution"] == "COMPILER_EXACT"
        && normalized["compilerResolution"] == "PROVEN"
        && normalized["provider"] == "K2_FIR"
        && normalized["compilerSchema"] == "declaration-relation/0.1"
        && normalized["sourceProvenance"] == "COMPILER_UTF16_RANGE_TO_UTF8_BYTES"
        && owner.normalized["compilerCallableId"] == source_callable
        && owner
            .normalized
            .get("jvmDescriptor")
            .is_none_or(|d| d == source_descriptor)
        && source == owner.symbol
        && format!("callable:{target_callable}#jvm:{target_descriptor}") == target
        && crate::semantic_validation::validate_kotlin_full_symbol_identity(&source).is_ok()
        && crate::semantic_validation::validate_kotlin_full_symbol_identity(target).is_ok()
}

fn project_argument_bindings(
    raw: Option<&serde_json::Value>,
    site: &serde_json::Value,
    target_descriptor: &str,
    source: &Source,
    neutral_site: &NeutralExactCallSite,
) -> (
    Option<NeutralCallArgumentBindings>,
    Vec<super::super::model::Gap>,
) {
    let Some(raw) = raw else {
        return (None, Vec::new());
    };
    let gap_for =
        |code: &str, detail: &str| super::gap(code, detail, Some(neutral_site.citation_id.clone()));
    let unsupported = || {
        let gap = gap_for(
            "ARGUMENT_BINDINGS_UNSUPPORTED",
            "The retained compiler argument-binding schema is not supported; the exact call site remains available.",
        );
        (
            Some(NeutralCallArgumentBindings {
                schema: raw["schema"].as_str().unwrap_or_default().to_owned(),
                arguments: Vec::new(),
                omitted_default_parameter_indices: Vec::new(),
                gaps: vec![gap.clone()],
            }),
            vec![gap],
        )
    };
    match raw.get("schema").and_then(serde_json::Value::as_str) {
        Some(ARGUMENT_BINDINGS_SCHEMA) => {}
        Some(_) => return unsupported(),
        None => {
            return rejected_argument_bindings(
                raw,
                &gap_for,
                "The retained compiler argument-binding schema is missing.",
            );
        }
    }
    let valid_shape = raw.as_object().is_some_and(|object| {
        object.keys().all(|key| {
            matches!(
                key.as_str(),
                "schema" | "argumentToParameter" | "omittedDefaultParameterIndices"
            )
        })
    });
    let Some(arguments) = raw.get("argumentToParameter") else {
        return rejected_argument_bindings(
            raw,
            &gap_for,
            "The retained compiler argument mapping is missing.",
        );
    };
    let Some(omitted) = raw.get("omittedDefaultParameterIndices") else {
        return rejected_argument_bindings(
            raw,
            &gap_for,
            "The retained omitted-default parameter set is missing.",
        );
    };
    let (Some(call_start), Some(call_end)) = (site["byteStart"].as_u64(), site["byteEnd"].as_u64())
    else {
        return rejected_argument_bindings(
            raw,
            &gap_for,
            "The exact call-site range is unavailable.",
        );
    };
    if !valid_shape
        || crate::semantic_validation::validate_kotlin_call_argument_bindings(
            call_start,
            call_end,
            target_descriptor,
            arguments,
            omitted,
        )
        .is_err()
    {
        return rejected_argument_bindings(
            raw,
            &gap_for,
            "The retained compiler argument mapping is malformed or does not match the exact target.",
        );
    }
    let Some(rows) = arguments.as_array() else {
        return rejected_argument_bindings(
            raw,
            &gap_for,
            "The retained compiler argument mapping is not an array.",
        );
    };
    let Some(omitted_rows) = omitted.as_array() else {
        return rejected_argument_bindings(
            raw,
            &gap_for,
            "The retained omitted-default parameter set is not an array.",
        );
    };
    let mut projected = Vec::with_capacity(rows.len());
    let mut previous_end = 0_usize;
    for row in rows {
        let (Some(start), Some(end)) = (row["argumentStart"].as_u64(), row["argumentEnd"].as_u64())
        else {
            return rejected_argument_bindings(
                raw,
                &gap_for,
                "A retained argument source range is incomplete.",
            );
        };
        let Some(relative_start) = start
            .checked_sub(call_start)
            .and_then(|offset| usize::try_from(offset).ok())
        else {
            return rejected_argument_bindings(
                raw,
                &gap_for,
                "A retained argument begins outside its call site.",
            );
        };
        let Some(relative_end) = end
            .checked_sub(call_start)
            .and_then(|offset| usize::try_from(offset).ok())
        else {
            return rejected_argument_bindings(
                raw,
                &gap_for,
                "A retained argument ends outside its call site.",
            );
        };
        if relative_start < previous_end || relative_start >= relative_end {
            return rejected_argument_bindings(
                raw,
                &gap_for,
                "Retained argument source spans overlap or are empty.",
            );
        }
        let Some(expression) = source.text.get(relative_start..relative_end) else {
            return rejected_argument_bindings(
                raw,
                &gap_for,
                "A retained argument source span is not a UTF-8 boundary in the exact call site.",
            );
        };
        let (Some(argument_type), Some(parameter), Some(parameter_index), Some(parameter_type)) = (
            row["argumentType"].as_str(),
            row["parameter"].as_str(),
            row["parameterIndex"].as_u64(),
            row["parameterType"].as_str(),
        ) else {
            return rejected_argument_bindings(
                raw,
                &gap_for,
                "A retained argument mapping lacks typed formal-parameter metadata.",
            );
        };
        projected.push(NeutralCallArgumentBinding {
            compilation_byte_start: start,
            compilation_byte_end: end,
            argument_name: row["argumentName"].as_str().map(str::to_owned),
            argument_type: argument_type.to_owned(),
            parameter: parameter.to_owned(),
            parameter_index,
            parameter_type: parameter_type.to_owned(),
            expression: expression.to_owned(),
            citation_id: citation_id(source, relative_start, relative_end),
        });
        previous_end = relative_end;
    }
    let omitted_default_parameter_indices = omitted_rows
        .iter()
        .filter_map(serde_json::Value::as_u64)
        .collect::<Vec<_>>();
    (
        Some(NeutralCallArgumentBindings {
            schema: ARGUMENT_BINDINGS_SCHEMA.to_owned(),
            arguments: projected,
            omitted_default_parameter_indices,
            gaps: Vec::new(),
        }),
        Vec::new(),
    )
}

fn rejected_argument_bindings(
    raw: &serde_json::Value,
    gap_for: &impl Fn(&str, &str) -> super::super::model::Gap,
    detail: &str,
) -> (
    Option<NeutralCallArgumentBindings>,
    Vec<super::super::model::Gap>,
) {
    let gap = gap_for("ARGUMENT_BINDINGS_REJECTED", detail);
    (
        Some(NeutralCallArgumentBindings {
            schema: raw["schema"].as_str().unwrap_or_default().to_owned(),
            arguments: Vec::new(),
            omitted_default_parameter_indices: Vec::new(),
            gaps: vec![gap.clone()],
        }),
        vec![gap],
    )
}

fn argument_source_range(
    site: &NeutralExactCallSite,
    argument: &NeutralCallArgumentBinding,
) -> Option<(usize, usize)> {
    let start = argument
        .compilation_byte_start
        .checked_sub(site.compilation_byte_start)
        .and_then(|offset| usize::try_from(offset).ok())?;
    let end = argument
        .compilation_byte_end
        .checked_sub(site.compilation_byte_start)
        .and_then(|offset| usize::try_from(offset).ok())?;
    (start < end).then_some((start, end))
}

fn valid_source(service: &str, revision: &str, source: &Source) -> bool {
    let transformed = crate::generation_service::TRANSFORMED_SOURCE_AUTHORITY;
    (source.authority == "EXACT_SNAPSHOT_TEXT" || source.authority == transformed)
        && (source.authority != transformed || source.url.is_none())
        && source.service == service
        && source.revision == revision
        && !source.file.is_empty()
        && !source.text.is_empty()
        && source.start_line > 0
        && source.end_line >= source.start_line
        && source.end_line - source.start_line + 1 == source.text.lines().count() as u64
        && source.text_digest == crate::canonical::hash_bytes(source.text.as_bytes())
        && source.occurrence.as_ref().is_none_or(|occurrence| {
            occurrence.end_byte.checked_sub(occurrence.start_byte) == Some(source.text.len())
        })
        && canonical_sha256(&source.evidence_digest)
}

fn canonical_sha256(value: &str) -> bool {
    value.strip_prefix("sha256:").is_some_and(|hex| {
        hex.len() == 64
            && hex
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    })
}

fn citation_id(source: &Source, start: usize, end: usize) -> String {
    format!(
        "citation-{}",
        &crate::canonical::hash_bytes(
            format!("{}:{start}:{end}:{}", source.id, source.text_digest).as_bytes()
        )[7..31]
    )
}

pub(in crate::documentation::static_pages) fn expected_citation(source: &Source) -> Citation {
    expected_citation_range(source, 0, source.text.len())
        .expect("full retained source range must be valid UTF-8")
}

fn expected_citation_range(source: &Source, start: usize, end: usize) -> Option<Citation> {
    let text = source.text.get(start..end)?;
    let start_line = source.start_line
        + source.text.as_bytes()[..start]
            .iter()
            .filter(|byte| **byte == b'\n')
            .count() as u64;
    let end_line = source.start_line
        + source.text.as_bytes()[..end.saturating_sub(1).max(start)]
            .iter()
            .filter(|byte| **byte == b'\n')
            .count() as u64;
    let url = source.url.as_ref().map(|url| {
        format!(
            "{}#L{start_line}-L{end_line}",
            url.split('#').next().unwrap_or(url)
        )
    });
    Some(Citation {
        id: citation_id(source, start, end),
        source_id: source.id.clone(),
        service: source.service.clone(),
        revision: source.revision.clone(),
        file: source.file.clone(),
        start_line,
        end_line,
        start_byte: start,
        end_byte: end,
        text_digest: crate::canonical::hash_bytes(text.as_bytes()),
        evidence_digest: source.evidence_digest.clone(),
        authority: source.authority.clone(),
        url,
    })
}

pub(in crate::documentation::static_pages) fn validate_page(
    page: &super::super::model::PageContent,
    callable: &CallableProjection,
) -> Result<(), ClewError> {
    validate_node(
        &page.selection.service,
        &page.service_revision,
        &page.observations,
        &page.sources,
        &page.citations,
        callable,
    )
}

pub(in crate::documentation::static_pages) fn validate_node(
    service: &str,
    revision: &str,
    observations: &BTreeMap<String, Observation>,
    sources: &BTreeMap<String, Source>,
    citations: &BTreeMap<String, Citation>,
    callable: &CallableProjection,
) -> Result<(), ClewError> {
    let Some(retained) = &callable.retained_call_sites else {
        return Ok(());
    };
    let owner = observations
        .get(&callable.declaration_id)
        .ok_or_else(|| invalid("retained Kotlin call-site owner is not retained"))?;
    if !super::compiler::admitted(owner) {
        return Err(invalid(
            "retained exact call sites require an admitted compiler declaration",
        ));
    }
    let scope = owner.normalized["scope"]
        .as_str()
        .ok_or_else(|| invalid("retained Kotlin call-site owner scope is missing"))?;
    let owner_key = NeutralExactCallSiteOwnerKey {
        service: service.to_owned(),
        scope: scope.to_owned(),
        symbol: callable.symbol.clone(),
    };
    let owner_source = match owner.source_ids.as_slice() {
        [id] => sources.get(id).filter(|source| source.id == *id),
        _ => None,
    };
    let candidates = candidate_relations(observations, &callable.symbol, scope);
    let expected = derive_projection(
        service,
        revision,
        &owner_key,
        owner,
        owner_source,
        &candidates,
        sources,
        callable.citation_id.clone(),
    )?;
    let expected = augment_source_events(expected, owner, owner_source, observations, sources)?;
    if retained != &expected {
        return Err(invalid(
            "retained Kotlin call-site payload differs from its copied compiler evidence",
        ));
    }
    if expected.gaps.is_empty() {
        for site in &expected.sites {
            let source = sources
                .get(&site.source_id)
                .ok_or_else(|| invalid("retained Kotlin call-site source is missing"))?;
            if citations.get(&site.citation_id) != Some(&expected_citation(source)) {
                return Err(invalid(
                    "retained Kotlin call-site citation differs from its exact source text",
                ));
            }
            if let Some(bindings) = &site.argument_bindings
                && bindings.gaps.is_empty()
            {
                for argument in &bindings.arguments {
                    let (start, end) = argument_source_range(site, argument).ok_or_else(|| {
                        invalid("retained Kotlin argument citation range is invalid")
                    })?;
                    if citations.get(&argument.citation_id)
                        != expected_citation_range(source, start, end).as_ref()
                    {
                        return Err(invalid(
                            "retained Kotlin argument citation differs from its exact source subspan",
                        ));
                    }
                }
            }
        }
    } else if expected.gaps[0]
        .citation_id
        .as_deref()
        .is_none_or(|id| citations.get(id).is_none())
    {
        return Err(invalid(
            "retained Kotlin call-site limitation has no declaration citation",
        ));
    }
    Ok(())
}
