//! ASP.NET Core trigger metadata over sealed Roslyn declaration facts.
use crate::error::{ClewError, ErrorCode};
use serde_json::Value;

pub use clew_framework_aspnetcore::{AspNetCoreEntry, AspNetCoreMetadata, describe_trigger};

pub const CSHARP_FACTS_DOMAIN: &str = "analysis:csharp-compiler-facts";
pub const ATTRIBUTE_AUTHORITY: &str = clew_facts::CLR_ATTRIBUTE_AUTHORITY;

/// Derive framework metadata from a C# declaration's portable attribute facts.
/// Declarations without `clrAttributes` are outside the framework's input.
pub fn metadata_for_fact(fact: &Value) -> Result<Option<AspNetCoreMetadata>, ClewError> {
    let Some(input) = fact.get("clrAttributes") else {
        return Ok(None);
    };
    let input: clew_facts::ClrAttributeFacts = serde_json::from_value(input.clone())
        .map_err(|_| invalid("CLR attribute facts violate their closed contract"))?;
    if Some(input.declaration.as_str()) != fact.get("symbolIdentity").and_then(Value::as_str) {
        return Err(invalid(
            "CLR attribute facts differ from their compiler declaration",
        ));
    }
    clew_framework_aspnetcore::analyze(&input)
        .map(Some)
        .map_err(invalid)
}

/// Minimal APIs register endpoints in code; attribute facts cannot observe them.
pub fn registers_minimal_api(fact: &Value) -> bool {
    fact.get("relationKind").and_then(Value::as_str) == Some("CALLS")
        && fact
            .get("targetIdentity")
            .and_then(Value::as_str)
            .is_some_and(|target| {
                target.starts_with(
                    "method:class:Microsoft.AspNetCore.Builder.EndpointRouteBuilderExtensions#Map",
                )
            })
}

fn invalid(message: impl Into<String>) -> ClewError {
    ClewError::new(ErrorCode::InvalidInput, message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn declaration_identity_binds_the_attribute_input() {
        let identity = "method:class:Orders.OrdersController#Get()V";
        let mut fact = json!({"symbolIdentity":identity,"clrAttributes":{
            "schema":clew_facts::CLR_ATTRIBUTE_SCHEMA,"authority":ATTRIBUTE_AUTHORITY,"declaration":identity,
            "method":{"name":"Get","accessibility":"PUBLIC","isStatic":false,"isAbstract":false,"isGeneric":false,"isOverride":false,
                "attributes":[{"attributeType":"class:Microsoft.AspNetCore.Mvc.HttpGetAttribute",
                    "attributeBases":["class:Microsoft.AspNetCore.Mvc.Routing.HttpMethodAttribute"],
                    "attributeInterfaces":["class:Microsoft.AspNetCore.Mvc.Routing.IActionHttpMethodProvider","class:Microsoft.AspNetCore.Mvc.Routing.IRouteTemplateProvider"],
                    "constructorArguments":[{"kind":"PRIMITIVE","type":"string","value":"orders"}],"namedArguments":{},"constructorParameters":["template"]}]},
            "containingType":{"identity":"class:Orders.OrdersController","name":"OrdersController","accessibility":"PUBLIC",
                "isAbstract":false,"isGeneric":false,"isNested":false,"attributes":[],"baseTypes":[]},
            "overriddenAttributes":[],"boundaries":[],"coverage":{"status":"COMPLETE","scope":clew_facts::CLR_ATTRIBUTE_SCOPE}
        }});
        let metadata = metadata_for_fact(&fact).unwrap().unwrap();
        assert_eq!(metadata.entries.len(), 1);
        assert_eq!(
            describe_trigger(&metadata.entries[0])["paths"],
            json!(["/orders"])
        );
        fact["symbolIdentity"] = json!("method:class:Orders.Other#Get()V");
        assert!(metadata_for_fact(&fact).is_err());
        assert!(
            metadata_for_fact(&json!({"symbolIdentity":identity}))
                .unwrap()
                .is_none()
        );
    }
}
