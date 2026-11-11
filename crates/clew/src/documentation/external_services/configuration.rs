//! Authored static references over exact selected source, never resolved values.
#[cfg(test)]
use super::{Check, invalid, store};
#[cfg(not(test))]
use super::{Check, invalid, store};
use crate::error::ClewError;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeSet;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConfigurationReference {
    pub id: String,
    pub key: String,
    pub binding: String,
    /// Source/profile scope as authored; it does not select an active profile.
    pub scope: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reference: Option<StaticReference>,
    pub sources: Vec<SourceReference>,
    pub declaration: crate::documentation::model::Declaration,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StaticReference {
    pub kind: String,
    pub name: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SourceReference {
    pub service: String,
    pub source_id: String,
    pub text_digest: String,
}
fn bounded(value: &str, max: usize) -> bool {
    !value.trim().is_empty() && value.len() <= max && !value.chars().any(char::is_control)
}
pub(super) fn validate(rows: &[ConfigurationReference]) -> Result<(), ClewError> {
    let mut ids = BTreeSet::new();
    if rows.len() > 64 {
        return Err(invalid("too many configuration references"));
    }
    for row in rows {
        if !store::valid_id(&row.id)
            || !ids.insert(&row.id)
            || !bounded(&row.key, 512)
            || !bounded(&row.scope, 2048)
            || !matches!(
                row.binding.as_str(),
                "static-reference" | "unresolved-reference" | "explicitly-unconfigured"
            )
            || row.sources.is_empty()
            || row.sources.len() > 8
            || !matches!(
                row.declaration.origin.as_str(),
                "human" | "agent-proposal" | "imported"
            )
            || !bounded(&row.declaration.rationale, 4096)
        {
            return Err(invalid(
                "configuration references need bounded static names, scope, provenance and source bindings",
            ));
        }
        if let Some(reference) = &row.reference {
            if !matches!(
                reference.kind.as_str(),
                "environment-placeholder" | "property-placeholder" | "code-symbol"
            ) || !bounded(&reference.name, 512)
                || (reference.kind == "environment-placeholder"
                    && !reference
                        .name
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'_'))
                || row.binding == "explicitly-unconfigured"
            {
                return Err(invalid(
                    "configuration reference names describe static references, never resolved values",
                ));
            }
        } else if row.binding == "static-reference" {
            return Err(invalid("static-reference requires a named reference"));
        }
        let mut sources = BTreeSet::new();
        for source in &row.sources {
            if !store::valid_id(&source.service)
                || !bounded(&source.source_id, 512)
                || !source.text_digest.starts_with("sha256:")
                || source.text_digest.len() != 71
                || !source.text_digest[7..]
                    .bytes()
                    .all(|b| b.is_ascii_hexdigit())
                || !sources.insert((&source.service, &source.source_id))
            {
                return Err(invalid("invalid configuration source identity or digest"));
            }
        }
    }
    Ok(())
}
pub(super) fn project(checked: &Check, rows: &[ConfigurationReference]) -> Vec<Value> {
    rows.iter().map(|row|{
        let sources=row.sources.iter().map(|reference|{
            let source=checked.services.get(&reference.service).and_then(|e|e.sources.get(&reference.source_id));
            let state=match source {None=>"SOURCE_UNAVAILABLE",Some(s) if s.text_digest!=reference.text_digest=>"SOURCE_CHANGED",Some(_)=>"SOURCE_BOUND"};
            json!({"service":reference.service,"sourceId":reference.source_id,"textDigest":reference.text_digest,"state":state,
                "file":source.map(|s|&s.file),"startLine":source.map(|s|s.start_line),"endLine":source.map(|s|s.end_line),"revision":source.map(|s|&s.revision)})
        }).collect::<Vec<_>>();
        let bound=sources.iter().filter(|s|s["state"]=="SOURCE_BOUND").count();
        let changed=sources.iter().filter(|s|s["state"]=="SOURCE_CHANGED").count();
        let missing=sources.len()-bound-changed;
        let status=if bound==sources.len(){"SOURCE_BOUND"}else if missing==sources.len(){"SOURCE_UNAVAILABLE"}else if changed==sources.len(){"SOURCE_CHANGED"}else{"SOURCE_PARTIAL"};
        json!({"id":row.id,"key":row.key,"binding":row.binding,"scope":row.scope,"reference":row.reference,"sources":sources,"sourceAvailability":{"status":status,"bound":bound,"changed":changed,"missing":missing},"declaration":row.declaration,
            "authority":"AUTHORED_STATIC_REFERENCE","runtime":"UNVERIFIED"})
    }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    fn reference() -> ConfigurationReference {
        serde_json::from_value(json!({"id":"endpoint","key":"pricing.client.endpoint","binding":"static-reference","scope":"Base resource; effective profile unknown","reference":{"kind":"environment-placeholder","name":"PRICING_ENDPOINT"},"sources":[{"service":"owner","sourceId":"configuration-source","textDigest":format!("sha256:{}","a".repeat(64))}],"declaration":{"origin":"agent-proposal","rationale":"The selected resource references an environment variable name; no value is read."}})).unwrap()
    }
    #[test]
    fn static_configuration_references_reject_values_and_unbound_runtime_claims() {
        let row = reference();
        validate(std::slice::from_ref(&row)).unwrap();
        let mut value = serde_json::to_value(&row).unwrap();
        value["resolvedValue"] = json!("not-a-reference");
        assert!(serde_json::from_value::<ConfigurationReference>(value).is_err());
        let mut invalid_row = row.clone();
        invalid_row.binding = "runtime-enabled".into();
        assert!(validate(&[invalid_row]).is_err());
        let mut invalid_row = row.clone();
        invalid_row.sources.clear();
        assert!(validate(&[invalid_row]).is_err());
        let mut invalid_row = row.clone();
        invalid_row.reference.as_mut().unwrap().name = "${PRICING_ENDPOINT}".into();
        assert!(validate(&[invalid_row]).is_err());
        assert!(validate(&[row.clone(), row]).is_err());
    }
    #[test]
    fn configuration_projection_preserves_profiles_and_does_not_promote_missing_or_changed_source()
    {
        let row = reference();
        let mut alternate = row.clone();
        alternate.id = "production".into();
        alternate.scope = "application-prod.yml; activation unknown".into();
        alternate.reference.as_mut().unwrap().name = "PRICING_PROD_ENDPOINT".into();
        let mut checked:Check=serde_json::from_value(json!({"schema":"fixture","inputDigest":"input","contextDigest":"context","services":{},"unresolved":{},"interactions":{},"scenarios":{},"dependencies":{}})).unwrap();
        let missing = project(&checked, &[row.clone(), alternate.clone()]);
        assert_eq!(missing.len(), 2);
        assert_eq!(missing[0]["sources"][0]["state"], "SOURCE_UNAVAILABLE");
        assert_eq!(
            missing[0]["sourceAvailability"]["status"],
            "SOURCE_UNAVAILABLE"
        );

        assert_ne!(missing[0]["reference"], missing[1]["reference"]);
        let source = json!({"id":"configuration-source","service":"owner","revision":"selected-revision","file":"src/main/resources/application.yml","startLine":1,"endLine":3,"text":"endpoint: ${PRICING_ENDPOINT}","textDigest":row.sources[0].text_digest,"evidenceDigest":"evidence","authority":"EXACT_SNAPSHOT_TEXT","url":null});
        checked.services.insert("owner".into(),serde_json::from_value(json!({"schema":"fixture","service":"owner","revision":"selected-revision","serviceDigest":"service","extractor":"fixture","runtimeMode":"SOURCE","coverage":"SYNTAX","boundaries":[],"entrypoints":[],"observations":{},"sources":{"configuration-source":source},"contracts":{}})).unwrap());
        let bound = project(&checked, std::slice::from_ref(&row));
        assert_eq!(bound[0]["sources"][0]["state"], "SOURCE_BOUND");
        assert_eq!(bound[0]["sourceAvailability"]["status"], "SOURCE_BOUND");
        assert_eq!(bound[0]["runtime"], "UNVERIFIED");

        let mut mixed_row = row.clone();
        mixed_row.sources.push(SourceReference {
            service: "owner".into(),
            source_id: "missing-resource".into(),
            text_digest: row.sources[0].text_digest.clone(),
        });
        let mixed = project(&checked, &[mixed_row]);
        assert_eq!(
            mixed[0]["sourceAvailability"],
            json!({"status":"SOURCE_PARTIAL","bound":1,"missing":1,"changed":0})
        );

        checked
            .services
            .get_mut("owner")
            .unwrap()
            .sources
            .get_mut("configuration-source")
            .unwrap()
            .text_digest = format!("sha256:{}", "b".repeat(64));
        let changed = project(&checked, &[row]);
        assert_eq!(changed[0]["sources"][0]["state"], "SOURCE_CHANGED");

        assert!(
            !serde_json::to_string(&changed)
                .unwrap()
                .contains("resolvedValue")
        );
    }
}
