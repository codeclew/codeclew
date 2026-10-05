//! Project Roslyn facts onto the declaration shapes documentation consumes.
//!
//! C# facts already use the portable JVM identity grammar. Documentation models
//! data members as fields and types as classes, so properties become `FIELD`
//! declarations (identity prefix `field:`) and structs/delegates become `CLASS`.
//! CLR-specific detail stays in the projected fact; the signature erasure into
//! descriptor grammar is recorded as an explicit boundary.
use crate::error::{ClewError, ErrorCode};
use serde_json::{Value, json};

pub(super) const CSHARP_FACT_SCHEMA: &str = crate::csharp_adapter_v2::CSHARP_FACT_SCHEMA;
pub(super) const CSHARP_FACTS_DOMAIN: &str =
    crate::csharp_adapter_v2::CSHARP_COMPILER_FACTS_CAPABILITY;

pub(super) fn project_facts(
    facts: Vec<(Value, String)>,
) -> Result<Vec<(Value, String)>, ClewError> {
    let mut output = Vec::with_capacity(facts.len() + 1);
    let mut declarations = 0usize;
    for (mut fact, binding) in facts {
        match fact["kind"].as_str() {
            Some("DECLARATION") => {
                let kind = match fact["declarationKind"].as_str() {
                    Some("PROPERTY") => "FIELD",
                    Some("STRUCT" | "DELEGATE") => "CLASS",
                    Some("EVENT" | "NAMESPACE") | None => continue,
                    Some(other) => other,
                }
                .to_owned();
                fact["declarationKind"] = json!(kind);
                if let Some(identity) = fact["symbolIdentity"].as_str() {
                    fact["symbolIdentity"] = json!(portable_member(identity));
                }
                declarations += 1;
                output.push((fact, binding));
            }
            Some("RELATION") => {
                if let Some(identity) = fact["sourceIdentity"].as_str() {
                    fact["sourceIdentity"] = json!(portable_member(identity));
                }
                output.push((fact, binding));
            }
            Some("BOUNDARY") => output.push((fact, binding)),
            _ => {}
        }
    }
    if declarations == 0 {
        return Err(ClewError::new(
            ErrorCode::IncompleteSemanticAnalysis,
            "C# compiler declarations are unavailable; restore the selected projects before documenting behavior",
        ));
    }
    output.push((
        json!({"kind":"BOUNDARY","code":"CLR_SIGNATURE_PROJECTED_TO_JVM_DESCRIPTOR_GRAMMAR"}),
        crate::canonical::hash_bytes(CSHARP_FACT_SCHEMA.as_bytes()),
    ));
    Ok(output)
}

/// Properties are data members in documentation; their identity uses `field:`.
fn portable_member(identity: &str) -> String {
    match identity.strip_prefix("property:") {
        Some(rest) => format!("field:{rest}"),
        None => identity.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn properties_become_fields_and_types_keep_portable_identities() {
        let facts = vec![
            (
                json!({"schema":CSHARP_FACT_SCHEMA,"kind":"DECLARATION","declarationKind":"PROPERTY",
                    "symbolIdentity":"property:class:Orders.Dto#Id:I","ownerIdentity":"class:Orders.Dto","name":"Id"}),
                "a".to_owned(),
            ),
            (
                json!({"schema":CSHARP_FACT_SCHEMA,"kind":"DECLARATION","declarationKind":"STRUCT",
                    "symbolIdentity":"class:Orders.Point","ownerIdentity":"module:unnamed","name":"Point"}),
                "b".to_owned(),
            ),
            (
                json!({"schema":CSHARP_FACT_SCHEMA,"kind":"RELATION","relationKind":"CALLS",
                    "sourceIdentity":"property:class:Orders.Dto#Total:I","targetIdentity":"method:class:Orders.Dto#Sum()I"}),
                "c".to_owned(),
            ),
            (
                json!({"schema":CSHARP_FACT_SCHEMA,"kind":"SOURCE_FILE","file":"Dto.cs"}),
                "d".to_owned(),
            ),
        ];
        let projected = project_facts(facts).unwrap();
        assert_eq!(projected[0].0["declarationKind"], "FIELD");
        assert_eq!(
            projected[0].0["symbolIdentity"],
            "field:class:Orders.Dto#Id:I"
        );
        assert_eq!(projected[1].0["declarationKind"], "CLASS");
        assert_eq!(
            projected[2].0["sourceIdentity"],
            "field:class:Orders.Dto#Total:I"
        );
        assert_eq!(
            projected.last().unwrap().0["code"],
            "CLR_SIGNATURE_PROJECTED_TO_JVM_DESCRIPTOR_GRAMMAR"
        );
        assert_eq!(projected.len(), 4);
        assert!(project_facts(vec![]).is_err());
    }
}
