//! Strict original-byte admission of the producer-normalized Kotlin input.
//! Parsing, compiler identity selection and argument binding happened during
//! capture; this consumer neither reparses source nor invents Java call rows.
use super::*;
use crate::semantic_validation::KotlinPropertyStorageProof as StorageProof;
use crate::{
    canonical::hash_bytes,
    documentation::static_pages::model::SourceCallNode,
    documentation::{digest, invalid, model::ServiceEvidence},
};
use serde::Deserialize;
use std::collections::BTreeSet;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Envelope {
    schema: String,
    authority: String,
    coordinate_domain: String,
    owner_symbol_identity: String,
    owner_byte_start: usize,
    owner_byte_end: usize,
    file: String,
    compilation_scope: String,
    full_compilation_source_digest: String,
    body: usize,
    nodes: Vec<RawNode>,
    variables: Vec<RawVariable>,
    #[serde(default)]
    members: Vec<RawMember>,
    boundaries: Vec<String>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawNode {
    kind: Kind,
    byte_start: usize,
    byte_end: usize,
    children: Vec<usize>,
    roles: BTreeMap<Role, usize>,
    actuals: Vec<RawActual>,
    default_arguments: Vec<usize>,
    target_identity: Option<String>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawActual {
    expression: usize,
    formal_slot: Option<usize>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawVariable {
    schema: String,
    owner_symbol_identity: String,
    variable_identity: String,
    variable_kind: String,
    kind: String,
    name: String,
    parameter_index: Option<usize>,
    has_default: Option<bool>,
    variable_type: Option<String>,
    access_mode: Option<String>,
    resolution: String,
    authority: String,
    byte_start: usize,
    byte_end: usize,
    declaration_byte_start: usize,
    declaration_byte_end: usize,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawMember {
    schema: String,
    owner_symbol_identity: String,
    property_identity: String,
    access_mode: String,
    variable_type: String,
    byte_start: usize,
    byte_end: usize,
    property_byte_start: usize,
    property_byte_end: usize,
    property_file: String,
    compilation_scope: String,
    storage_proof: StorageProof,
}

pub(in super::super) fn prepare<'a>(
    evidence: &'a ServiceEvidence,
    node: &SourceCallNode,
) -> Result<Prepared<'a>, ClewError> {
    let owner = &evidence.observations[&node.callable.declaration_id];
    if owner.normalized["receiverType"].is_object()
        || owner.normalized["contextParameters"]
            .as_array()
            .is_some_and(|rows| !rows.is_empty())
    {
        return Ok(Prepared::VariablesUnavailable);
    }
    let Some(raw) = owner.normalized["documentation"].get("dataInput") else {
        return Ok(Prepared::VariablesUnavailable);
    };
    if serde_json::to_vec(raw)
        .map_err(|_| invalid("data input encoding failed"))?
        .len()
        > super::super::MAX_BYTES
    {
        return Err(invalid(
            "expandDataState exceeds bounded Kotlin input budget; narrow the selection",
        ));
    }
    let envelope: Envelope = serde_json::from_value(raw.clone())
        .map_err(|_| invalid("Kotlin data input violates its closed typed contract"))?;
    let site = &owner.normalized["outlineOwnerSource"];
    let source = site["sourceId"]
        .as_str()
        .and_then(|id| evidence.sources.get(id))
        .ok_or_else(|| invalid("Kotlin data input original source is unavailable"))?;
    if owner.digest != digest(&owner.normalized)?
        || owner.service != evidence.service
        || source.service != evidence.service
        || source.revision != evidence.revision
        || source.id != site["sourceId"]
        || owner.source_ids != [source.id.clone()]
        || site["sourceStatus"] != "SOURCE_RETAINED"
        || site["sourceDigest"] != source.text_digest
        || site["evidenceDigest"] != source.evidence_digest
        || source.text_digest != hash_bytes(source.text.as_bytes())
        || envelope.schema != "codeclew-kotlin-documentation-data/1.0"
        || envelope.authority != "KOTLIN_PSI_WITH_K2_VARIABLE_IDENTITIES"
        || envelope.coordinate_domain != "ORIGINAL_UTF8_BYTES"
        || envelope.owner_symbol_identity != owner.symbol
        || envelope.compilation_scope != node.scope
        || envelope.file != source.file
        || site["file"] != source.file
        || site["byteStart"].as_u64() != Some(envelope.owner_byte_start as u64)
        || site["byteEnd"].as_u64() != Some(envelope.owner_byte_end as u64)
        || site["fullCompilationSourceDigest"] != envelope.full_compilation_source_digest
        || envelope
            .owner_byte_end
            .checked_sub(envelope.owner_byte_start)
            != Some(source.text.len())
        || owner.normalized["analysis"]["analyzerCompilerVersion"] != "2.4.10"
        || envelope.nodes.is_empty()
        || envelope.nodes.len() > super::super::MAX_ROWS
        || envelope.variables.len() + envelope.members.len() > super::super::MAX_ROWS
        || envelope.body >= envelope.nodes.len()
        || envelope.boundaries.len() > 32
        || envelope.boundaries.iter().any(|code| {
            matches!(
                code.as_str(),
                "DOCUMENTATION_VARIABLE_BUDGET" | "DOCUMENTATION_VARIABLE_SOURCE_UNAVAILABLE"
            )
        })
    {
        return Err(invalid(
            "Kotlin data input owner, producer or original-source pins disagree",
        ));
    }
    let local = |start: usize, end: usize| -> Result<(usize, usize), ClewError> {
        let start = start
            .checked_sub(envelope.owner_byte_start)
            .ok_or_else(|| invalid("Kotlin data source span precedes its owner"))?;
        let end = end
            .checked_sub(envelope.owner_byte_start)
            .ok_or_else(|| invalid("Kotlin data source span precedes its owner"))?;
        if start >= end || source.text.get(start..end).is_none() {
            return Err(invalid("Kotlin data span is not retained original UTF-8"));
        }
        Ok((start, end))
    };
    let mut input = Input {
        source,
        nodes: vec![],
        body: Some(envelope.body),
        variables: BTreeMap::new(),
        calls: BTreeMap::new(),
        missing_sites: false,
    };
    let mut references = 0;
    for (index, raw) in envelope.nodes.iter().enumerate() {
        let range = local(raw.byte_start, raw.byte_end)?;
        if matches!(
            raw.kind,
            Kind::Field | Kind::Super | Kind::Construct | Kind::Throw | Kind::NestedBody
        ) {
            return Err(invalid(
                "Kotlin data syntax promotes an unsupported operation",
            ));
        }
        let links: Vec<_> = raw
            .children
            .iter()
            .copied()
            .chain(raw.roles.values().copied())
            .chain(raw.actuals.iter().map(|a| a.expression))
            .collect();
        references += links.len();
        if references > 16384
            || links.iter().any(|&child| {
                child >= index
                    || envelope.nodes[child].byte_start < raw.byte_start
                    || envelope.nodes[child].byte_end > raw.byte_end
            })
        {
            return Err(invalid(
                "Kotlin data syntax has cyclic, unbounded or outside-parent links",
            ));
        }
        let slots: BTreeSet<_> = raw.actuals.iter().filter_map(|a| a.formal_slot).collect();
        let defaults: BTreeSet<_> = raw.default_arguments.iter().copied().collect();
        if slots.len()
            != raw
                .actuals
                .iter()
                .filter(|a| a.formal_slot.is_some())
                .count()
            || defaults.len() != raw.default_arguments.len()
            || !slots.is_disjoint(&defaults)
            || (raw.kind != Kind::Call
                && (!raw.actuals.is_empty()
                    || !defaults.is_empty()
                    || raw.target_identity.is_some()))
        {
            return Err(invalid("Kotlin data call has contradictory formal slots"));
        }
        if raw.kind == Kind::Call {
            let candidates: Vec<_> = node
                .calls
                .iter()
                .filter_map(|edge| edge.exact_call_site.as_ref().map(|site| (edge, site)))
                .filter(|(_, site)| {
                    site.compilation_byte_start == raw.byte_start as u64
                        && site.compilation_byte_end == raw.byte_end as u64
                        && site.file == source.file
                        && site.full_compilation_source_digest
                            == envelope.full_compilation_source_digest
                        && Some(site.target_identity.as_str()) == raw.target_identity.as_deref()
                        && source.text.get(range.0..range.1) == Some(site.expression.as_str())
                })
                .collect();
            if let [(edge, site)] = candidates.as_slice() {
                let binding = BoundCall {
                    occurrence: format!("compiler-site/{}", site.relation_id),
                    target_node: edge.target_node.clone(),
                    status: edge.status.clone(),
                };
                if input.calls.insert(range, binding).is_some() {
                    return Err(invalid("Kotlin data call source occurrence is ambiguous"));
                }
            }
        }
        input.nodes.push(SyntaxNode {
            kind: raw.kind,
            range,
            children: raw.children.clone(),
            roles: raw.roles.clone(),
            actuals: raw
                .actuals
                .iter()
                .map(|a| Actual {
                    expression: a.expression,
                    formal_slot: a.formal_slot,
                })
                .collect(),
            default_arguments: raw.default_arguments.clone(),
        });
    }
    let parameters = owner.normalized["parameterTypes"]
        .as_array()
        .ok_or_else(|| invalid("Kotlin data formal descriptors are unavailable"))?;
    let mut declarations = BTreeMap::new();
    for variable in &envelope.variables {
        let range = local(variable.byte_start, variable.byte_end)?;
        let declaration_range = local(
            variable.declaration_byte_start,
            variable.declaration_byte_end,
        )?;
        let parameter = variable.variable_kind == "PARAMETER";
        let kind = if parameter {
            VariableKind::Parameter
        } else {
            VariableKind::Local
        };
        let declaration = variable.kind == "VARIABLE_DECLARATION";
        if variable.schema != "kotlin-documentation-variable/1.0"
            || variable.owner_symbol_identity != owner.symbol
            || variable.resolution != "COMPILER_EXACT"
            || variable.authority != "K2_RESOLVED_VARIABLE_SYMBOL"
            || !matches!(
                variable.variable_kind.as_str(),
                "PARAMETER" | "LOCAL_VARIABLE"
            )
            || !matches!(
                variable.kind.as_str(),
                "VARIABLE_DECLARATION" | "VARIABLE_ACCESS"
            )
            || variable.name.is_empty()
            || variable
                .variable_type
                .as_ref()
                .is_some_and(|t| t.len() > 4096)
            || (declaration && (range != declaration_range || variable.access_mode.is_some()))
            || (!declaration && !matches!(variable.access_mode.as_deref(), Some("READ" | "WRITE")))
            || (!parameter
                && (variable.parameter_index.is_some()
                    || variable.has_default.is_some()
                    || !variable
                        .variable_identity
                        .starts_with(&format!("{}/local-source/", owner.symbol))))
        {
            return Err(invalid(
                "Kotlin data variable has unsupported or inconsistent compiler identity",
            ));
        }
        if parameter {
            let slot = variable
                .parameter_index
                .ok_or_else(|| invalid("Kotlin data formal slot is missing"))?;
            let descriptor = parameters
                .get(slot)
                .ok_or_else(|| invalid("Kotlin data formal slot is outside its descriptor"))?;
            if variable.variable_identity != format!("{}/parameter/{slot}", owner.symbol)
                || descriptor["index"].as_u64() != Some(slot as u64)
                || descriptor["hasDefault"].as_bool() != variable.has_default
            {
                return Err(invalid(
                    "Kotlin data formal identity differs from its compiler descriptor",
                ));
            }
        }
        if declaration
            && declarations
                .insert(variable.variable_identity.clone(), variable)
                .is_some()
        {
            return Err(invalid("Kotlin data variable declaration is ambiguous"));
        }
        if input
            .variables
            .insert(
                range,
                Variable {
                    identity: variable.variable_identity.clone(),
                    kind,
                    declaration,
                    member: None,
                    formal_slot: variable.parameter_index,
                },
            )
            .is_some()
        {
            return Err(invalid(
                "Kotlin data variable source occurrence is ambiguous",
            ));
        }
    }
    for variable in &envelope.variables {
        let Some(declaration) = declarations.get(&variable.variable_identity) else {
            return Err(invalid(
                "Kotlin data variable target declaration is not retained",
            ));
        };
        if declaration.byte_start != variable.declaration_byte_start
            || declaration.byte_end != variable.declaration_byte_end
            || declaration.variable_kind != variable.variable_kind
            || declaration.parameter_index != variable.parameter_index
            || declaration.has_default != variable.has_default
            || declaration.variable_type != variable.variable_type
            || declaration.name != variable.name
        {
            return Err(invalid(
                "Kotlin data variable access disagrees with its compiler declaration",
            ));
        }
    }
    for member in &envelope.members {
        let range = local(member.byte_start, member.byte_end)?;
        if member.schema != "kotlin-documentation-member/1.0"
            || member.owner_symbol_identity != owner.symbol
            || member.compilation_scope != node.scope
            || !matches!(member.access_mode.as_str(), "READ" | "WRITE")
            || member.variable_type.is_empty()
            || member.variable_type.len() > 4096
            || !member.storage_proof.admitted()
        {
            return Err(invalid(
                "Kotlin property access lacks an ordinary backing-storage proof",
            ));
        }
        let properties: Vec<_> = evidence
            .observations
            .values()
            .filter(|property| {
                property.kind == "SYMBOL"
                    && property.service == evidence.service
                    && property.symbol == member.property_identity
                    && property.normalized["symbolIdentity"] == member.property_identity
                    && property.normalized["scope"] == node.scope
                    && property.normalized["schema"] == "declaration-descriptor/0.1"
                    && property.normalized["provider"] == "K2_FIR"
                    && property.normalized["resolution"] == "PROVEN"
                    && property.normalized["sourceProvenance"]
                        == "COMPILER_UTF16_RANGE_TO_UTF8_BYTES"
                    && property.normalized["compilerAuthority"] == "fir-facts-extractor/0.6"
                    && matches!(
                        property.normalized["declarationKind"].as_str(),
                        Some("PROPERTY" | "MUTABLE_PROPERTY")
                    )
            })
            .collect();
        let [property] = properties.as_slice() else {
            return Err(invalid(
                "Kotlin property compiler declaration is missing or ambiguous",
            ));
        };
        let proof: StorageProof =
            serde_json::from_value(property.normalized["documentationStorage"].clone())
                .map_err(|_| invalid("Kotlin property descriptor storage proof is missing"))?;
        let site = &property.normalized["outlineOwnerSource"];
        let source = site["sourceId"]
            .as_str()
            .and_then(|id| evidence.sources.get(id))
            .ok_or_else(|| invalid("Kotlin property original declaration source is missing"))?;
        if proof != member.storage_proof
            || property.digest != digest(&property.normalized)?
            || site["sourceStatus"] != "SOURCE_RETAINED"
            || site["file"] != member.property_file
            || source.file != member.property_file
            || source.service != evidence.service
            || source.revision != evidence.revision
            || property.source_ids != [source.id.clone()]
            || site["sourceDigest"] != source.text_digest
            || site["evidenceDigest"] != source.evidence_digest
            || source.text_digest != hash_bytes(source.text.as_bytes())
            || site["byteStart"].as_u64() != Some(member.property_byte_start as u64)
            || site["byteEnd"].as_u64() != Some(member.property_byte_end as u64)
            || member
                .property_byte_end
                .checked_sub(member.property_byte_start)
                != Some(source.text.len())
            || property.normalized["declaredType"] != member.variable_type
            || (member.access_mode == "WRITE"
                && property.normalized["declarationKind"] != "MUTABLE_PROPERTY")
        {
            return Err(invalid(
                "Kotlin property access and retained original storage declaration disagree",
            ));
        }
        let binding = Variable {
            identity: member.property_identity.clone(),
            kind: VariableKind::Property,
            declaration: false,
            formal_slot: None,
            member: Some(MemberStorage {
                declaration_id: property.id.clone(),
                static_member: property.normalized["ownerIdentity"]
                    .as_str()
                    .is_some_and(|id| id.starts_with("package:")),
            }),
        };
        if input.variables.insert(range, binding).is_some() {
            return Err(invalid(
                "Kotlin property occurrence collides with another storage binding",
            ));
        }
    }
    if input.formals()?.len() != parameters.len() {
        return Err(invalid("Kotlin data formal input closure is incomplete"));
    }
    Ok(Prepared::Body(input))
}
