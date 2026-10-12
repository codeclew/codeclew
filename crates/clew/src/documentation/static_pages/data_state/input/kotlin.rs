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
    #[serde(default)]
    accessor_attempts: Vec<RawMember>,
    #[serde(default)]
    string_constants: Vec<RawMember>,
    #[serde(default)]
    enum_entries: Vec<RawEnumEntry>,
    boundaries: Vec<String>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawEnumEntry {
    schema: String,
    owner_symbol_identity: String,
    enum_class_identity: String,
    entry_identity: String,
    entry_name: String,
    authority: String,
    resolution: String,
    compilation_scope: String,
    byte_start: usize,
    byte_end: usize,
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
    static_qualifier: Option<RawStaticQualifier>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawStaticQualifier {
    schema: String,
    authority: String,
    class_identity: String,
    byte_start: usize,
    byte_end: usize,
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
    string_constant: Option<String>,
    authority: Option<String>,
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
    let sites: Vec<_> = node
        .calls
        .iter()
        .filter_map(|edge| {
            edge.exact_call_site
                .as_ref()
                .map(|site| (site, edge.target_node.clone(), edge.status.clone()))
        })
        .collect();
    prepare_sites(evidence, &node.callable.declaration_id, &node.scope, &sites)
}

pub(in crate::documentation::static_pages) fn prepare_sites<'a>(
    evidence: &'a ServiceEvidence,
    declaration: &str,
    scope: &str,
    sites: &[(
        &crate::documentation::static_pages::model::NeutralExactCallSite,
        Option<String>,
        String,
    )],
) -> Result<Prepared<'a>, ClewError> {
    let owner = &evidence.observations[declaration];
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
        || !matches!(
            envelope.schema.as_str(),
            "codeclew-kotlin-documentation-data/1.0"
                | "codeclew-kotlin-documentation-data/1.1"
                | "codeclew-kotlin-documentation-data/1.2"
                | "codeclew-kotlin-documentation-data/1.3"
        )
        || (envelope.schema == "codeclew-kotlin-documentation-data/1.0"
            && !envelope.enum_entries.is_empty())
        || (!matches!(
            envelope.schema.as_str(),
            "codeclew-kotlin-documentation-data/1.2" | "codeclew-kotlin-documentation-data/1.3"
        ) && !envelope.accessor_attempts.is_empty())
        || (envelope.schema != "codeclew-kotlin-documentation-data/1.3"
            && !envelope.string_constants.is_empty())
        || envelope.authority != "KOTLIN_PSI_WITH_K2_VARIABLE_IDENTITIES"
        || envelope.coordinate_domain != "ORIGINAL_UTF8_BYTES"
        || envelope.owner_symbol_identity != owner.symbol
        || envelope.compilation_scope != scope
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
        || envelope.variables.len()
            + envelope.members.len()
            + envelope.accessor_attempts.len()
            + envelope.string_constants.len()
            + envelope.enum_entries.len()
            > super::super::MAX_ROWS
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
        enum_constants: BTreeMap::new(),
        accessors: BTreeMap::new(),
        string_constants: BTreeMap::new(),
        static_qualifiers: BTreeMap::new(),
        calls: BTreeMap::new(),
        missing_sites: false,
    };
    let mut references = 0;
    for (index, raw) in envelope.nodes.iter().enumerate() {
        let range = local(raw.byte_start, raw.byte_end)?;
        if raw.kind == Kind::Accessor
            && (raw.roles.keys().any(|r| *r != Role::Receiver)
                || raw.children != raw.roles.values().copied().collect::<Vec<_>>())
        {
            return Err(invalid(
                "Kotlin accessor syntax has unsupported evaluation children",
            ));
        }
        if matches!(
            raw.kind,
            Kind::Field | Kind::Super | Kind::Throw | Kind::NestedBody
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
            || (!matches!(raw.kind, Kind::Call | Kind::Construct)
                && (!raw.actuals.is_empty()
                    || !defaults.is_empty()
                    || raw.target_identity.is_some()))
        {
            return Err(invalid("Kotlin data call has contradictory formal slots"));
        }
        if matches!(raw.kind, Kind::Call | Kind::Construct) {
            let prefix = if raw.kind == Kind::Construct {
                "constructor:"
            } else {
                "callable:"
            };
            if raw.target_identity.as_deref().is_none_or(|target| {
                !target.starts_with(prefix)
                    || crate::semantic_validation::validate_kotlin_full_symbol_identity(target)
                        .is_err()
            }) {
                return Err(invalid(
                    "Kotlin invocation kind differs from its compiler target",
                ));
            }
            let candidates: Vec<_> = sites
                .iter()
                .filter(|(site, _, _)| {
                    site.compilation_byte_start == raw.byte_start as u64
                        && site.compilation_byte_end == raw.byte_end as u64
                        && site.file == source.file
                        && site.full_compilation_source_digest
                            == envelope.full_compilation_source_digest
                        && Some(site.target_identity.as_str()) == raw.target_identity.as_deref()
                        && source.text.get(range.0..range.1) == Some(site.expression.as_str())
                })
                .collect();
            if let [(site, target_node, status)] = candidates.as_slice() {
                let binding = BoundCall {
                    occurrence: format!("compiler-site/{}", site.relation_id),
                    target_node: target_node.clone(),
                    status: status.clone(),
                };
                if input.calls.insert(range, binding).is_some() {
                    return Err(invalid("Kotlin data call source occurrence is ambiguous"));
                }
            }
        }
        if let Some(qualifier) = &raw.static_qualifier {
            let span = local(qualifier.byte_start, qualifier.byte_end)?;
            if envelope.schema != "codeclew-kotlin-documentation-data/1.3"
                || raw.kind != Kind::Call
                || raw.roles.contains_key(&Role::Receiver)
                || qualifier.schema != "kotlin-documentation-static-qualifier/1.0"
                || qualifier.authority != "K2_STATIC_FUNCTION_RESOLVED_CLASS_QUALIFIER"
                || !input.calls.contains_key(&range)
                || span.0 != range.0
                || span.1 > range.1
                || qualifier
                    .class_identity
                    .strip_prefix("class:")
                    .is_none_or(|class| {
                        raw.target_identity
                            .as_deref()
                            .is_none_or(|target| !target.starts_with(&format!("callable:{class}.")))
                    })
                || input
                    .static_qualifiers
                    .insert(range, qualifier.class_identity.clone())
                    .is_some()
            {
                return Err(invalid(
                    "Kotlin static qualifier lacks an exact class, call or original-source binding",
                ));
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
    for entry in &envelope.enum_entries {
        let range = local(entry.byte_start, entry.byte_end)?;
        let class = entry
            .enum_class_identity
            .strip_prefix("class:")
            .filter(|class| {
                !class.is_empty() && class.len() <= 4096 && !class.chars().any(char::is_control)
            })
            .ok_or_else(|| invalid("Kotlin enum class identity is unavailable"))?;
        if entry.schema != "kotlin-documentation-enum-entry/1.0"
            || entry.owner_symbol_identity != owner.symbol
            || entry.compilation_scope != scope
            || entry.authority != "K2_RESOLVED_ENUM_ENTRY_SYMBOL"
            || entry.resolution != "COMPILER_EXACT"
            || entry.entry_name.is_empty()
            || entry.entry_name.len() > 4096
            || entry.entry_name.chars().any(char::is_control)
            || entry.entry_identity != format!("enum-entry:{class}.{}", entry.entry_name)
            || input
                .nodes
                .iter()
                .filter(|node| node.kind == Kind::EnumConstant && node.range == range)
                .count()
                != 1
        {
            return Err(invalid(
                "Kotlin enum access lacks a unique compiler-resolved entry",
            ));
        }
        let value = super::super::super::model::DataValue::EnumConstant {
            identity: entry.entry_identity.clone(),
            enum_class: entry.enum_class_identity.clone(),
            entry: entry.entry_name.clone(),
            authority: entry.authority.clone(),
        };
        if input.enum_constants.insert(range, value).is_some() {
            return Err(invalid("Kotlin enum occurrence is ambiguous"));
        }
    }
    if input.nodes.iter().any(|node| {
        node.kind == Kind::EnumConstant && !input.enum_constants.contains_key(&node.range)
    }) {
        return Err(invalid(
            "Kotlin enum syntax has no compiler-resolved identity",
        ));
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
    for (member, accessor, constant) in envelope
        .members
        .iter()
        .map(|m| (m, false, false))
        .chain(envelope.accessor_attempts.iter().map(|m| (m, true, false)))
        .chain(envelope.string_constants.iter().map(|m| (m, false, true)))
    {
        let range = local(member.byte_start, member.byte_end)?;
        if member.schema
            != if constant {
                "kotlin-documentation-string-constant/1.0"
            } else if accessor {
                "kotlin-documentation-accessor-attempt/1.0"
            } else {
                "kotlin-documentation-member/1.0"
            }
            || (accessor
                && member.authority.as_deref()
                    != Some("K2_RESOLVED_PROPERTY_SYMBOL_NOT_ACCESSOR_DISPATCH"))
            || (!accessor && !constant && member.authority.is_some())
            || (!constant && member.string_constant.is_some())
            || (constant
                && (member.authority.as_deref() != Some("K2_CONST_PROPERTY_LITERAL_STRING")
                    || member
                        .string_constant
                        .as_ref()
                        .is_none_or(|s| s.len() > 4096)
                    || member.variable_type != "kotlin/String"
                    || member.access_mode != "READ"))
            || member.owner_symbol_identity != owner.symbol
            || member.compilation_scope != scope
            || !matches!(member.access_mode.as_str(), "READ" | "WRITE")
            || member.variable_type.is_empty()
            || member.variable_type.len() > 4096
            || (!accessor && !constant && !member.storage_proof.admitted())
            || (accessor && !member.storage_proof.accessor_attempt_admitted())
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
                    && property.normalized["scope"] == scope
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
        if constant {
            let proof = &property.normalized["documentationStringConstant"];
            if proof["schema"] != "kotlin-documentation-string-constant-value/1.0"
                || property.normalized["declarationKind"] != "PROPERTY"
                || proof["authority"] != "K2_CONST_PROPERTY_LITERAL_STRING"
                || proof["isConst"] != true
                || proof["literalKind"] != "STRING"
                || proof["value"].as_str() != member.string_constant.as_deref()
                || input
                    .nodes
                    .iter()
                    .filter(|n| n.kind == Kind::StringConstant && n.range == range)
                    .count()
                    != 1
                || input.variables.contains_key(&range)
                || input.accessors.contains_key(&range)
                || input
                    .string_constants
                    .insert(
                        range,
                        BoundStringConstant {
                            declaration_id: property.id.clone(),
                            value: super::super::super::model::DataValue::StringConstant {
                                identity: member.property_identity.clone(),
                                value: member.string_constant.clone().unwrap(),
                                authority: "K2_CONST_PROPERTY_LITERAL_STRING".into(),
                            },
                        },
                    )
                    .is_some()
            {
                return Err(invalid(
                    "Kotlin string constant lacks its exact compiler declaration, value or occurrence",
                ));
            }
            continue;
        }
        if accessor {
            let indices: Vec<_> = input
                .nodes
                .iter()
                .enumerate()
                .filter(|(_, n)| n.kind == Kind::Accessor && n.range == range)
                .map(|(i, _)| i)
                .collect();
            let [index] = indices.as_slice() else {
                return Err(invalid(
                    "Kotlin accessor attempt lacks a unique syntax occurrence",
                ));
            };
            let write = input
                .nodes
                .iter()
                .any(|n| n.kind == Kind::Assignment && n.roles.get(&Role::Left) == Some(index));
            if write
                && input
                    .nodes
                    .iter()
                    .filter(|n| {
                        n.kind == Kind::Assignment && n.roles.get(&Role::Left) == Some(index)
                    })
                    .any(|n| {
                        n.roles
                            .get(&Role::Operator)
                            .is_none_or(|i| input.node(*i).text() != "=")
                    })
            {
                return Err(invalid(
                    "Kotlin accessor compound assignments are not admitted",
                ));
            }
            if write != (member.access_mode == "WRITE")
                || input.variables.contains_key(&range)
                || input
                    .accessors
                    .insert(
                        range,
                        BoundAccessor {
                            property_identity: member.property_identity.clone(),
                            declaration_id: property.id.clone(),
                            access_mode: member.access_mode.clone(),
                        },
                    )
                    .is_some()
            {
                return Err(invalid(
                    "Kotlin accessor attempt mode or occurrence is inconsistent",
                ));
            }
            continue;
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
    if input
        .nodes
        .iter()
        .any(|n| n.kind == Kind::Accessor && !input.accessors.contains_key(&n.range))
    {
        return Err(invalid(
            "Kotlin accessor syntax lacks a compiler property binding",
        ));
    }
    if input.nodes.iter().any(|n| {
        n.kind == Kind::StringConstant
            && (!input.string_constants.contains_key(&n.range)
                || !n.children.is_empty()
                || !n.roles.is_empty())
    }) {
        return Err(invalid(
            "Kotlin string constant syntax lacks an exact compiler binding",
        ));
    }
    if input.formals()?.len() != parameters.len() {
        return Err(invalid("Kotlin data formal input closure is incomplete"));
    }
    Ok(Prepared::Body(input))
}
