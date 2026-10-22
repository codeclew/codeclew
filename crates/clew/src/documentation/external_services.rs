//! Authored information about services whose source is not captured.
use super::{
    bytes,
    check::Check,
    cli::{InputArgs, ListArgs},
    digest, invalid, io_error,
    model::Observation,
    render::escape,
    store::{self, Repository},
};
use crate::error::ClewError;
use clap::Subcommand;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{collections::BTreeMap, path::PathBuf};

pub(super) const SCHEMA: &str = "codeclew-documentation-external-service/1.0";
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExternalService {
    pub schema: String,
    pub id: String,
    pub title: String,
    pub description: String,
    /// Authored HTTPS references; no automatic fetching or source authority.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repository: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wiki: Option<String>,
    pub declaration: super::model::Declaration,
    #[serde(default)]
    pub limitations: Vec<String>,
}
pub(super) fn validate(value: &ExternalService) -> Result<(), ClewError> {
    if value.schema != SCHEMA
        || !store::valid_id(&value.id)
        || value.title.trim().is_empty()
        || value.title.len() > 512
        || value.description.trim().is_empty()
        || value.description.len() > 16384
        || value.limitations.len() > 32
        || value.limitations.iter().any(|s| s.len() > 2048)
        || !matches!(
            value.declaration.origin.as_str(),
            "human" | "agent-proposal" | "imported"
        )
        || value.declaration.rationale.trim().is_empty()
        || value.declaration.rationale.len() > 4096
    {
        return Err(invalid(
            "invalid external-service identity, declaration or bounded authored information",
        ));
    }
    for url in [&value.repository, &value.wiki].into_iter().flatten() {
        if url.len() > 2048 || super::source_links::web_base(url).is_err() {
            return Err(invalid(
                "external-service links require clean HTTPS repository/wiki URLs without credentials, query or fragment",
            ));
        }
    }
    Ok(())
}
pub fn records(repo: &Repository) -> Result<BTreeMap<String, ExternalService>, ClewError> {
    let rows: BTreeMap<String, ExternalService> =
        repo.records("catalog/external-services", "json")?;
    for (id, value) in &rows {
        validate(value)?;
        if id != &value.id {
            return Err(invalid(
                "external-service filename does not match its identity",
            ));
        }
    }
    Ok(rows)
}
pub(super) fn validate_inputs(inputs: &super::store::RepositoryInputs) -> Result<(), ClewError> {
    if inputs.external_services.len() > store::MAX_RECORDS {
        return Err(invalid(
            "external-service registry exceeds its record bound",
        ));
    }
    for (id, value) in &inputs.external_services {
        validate(value)?;
        if id != &value.id || inputs.services.contains_key(id) {
            return Err(invalid(
                "external-service identity collides with a captured service or registry key",
            ));
        }
    }
    for interaction in inputs.interactions.values() {
        if let Some(id) = &interaction.external_service
            && (!interaction.external
                || interaction.to.service != *id
                || !inputs.external_services.contains_key(id))
        {
            return Err(invalid(
                "interaction externalService requires a registered external destination with the same to.service identity",
            ));
        }
    }
    Ok(())
}
pub(super) fn attach(
    inputs: &super::store::RepositoryInputs,
    checked: &mut Check,
) -> Result<(), ClewError> {
    validate_inputs(inputs)?;
    for (id, value) in &inputs.external_services {
        let normalized = json!({"externalService":value,"authority":"DECLARED_EXTERNAL_SERVICE","sourceCapture":"NOT_CAPTURED","runtime":"UNVERIFIED"});
        let key = format!("external-service:{id}");
        checked.dependencies.insert(
            key.clone(),
            Observation {
                id: key,
                kind: "EXTERNAL_SERVICE".into(),
                service: String::new(),
                symbol: value.title.clone(),
                digest: digest(&normalized)?,
                normalized,
                source_ids: vec![],
            },
        );
    }
    checked.refresh_digest()
}
#[derive(Debug, Subcommand)]
pub enum Command {
    Put(InputArgs),
    List(ListArgs),
    Show {
        #[arg(long)]
        root: PathBuf,
        #[arg(long)]
        id: String,
    },
    Remove(super::removal::Request),
}
pub fn run(command: Command) -> Result<Value, ClewError> {
    match command {
        Command::Put(args) => {
            let repo = Repository::open(&args.root)?;
            let value: ExternalService = store::read(&args.input, 64 * 1024)?;
            validate(&value)?;
            if repo.services()?.contains_key(&value.id) {
                return Err(invalid(
                    "external identity already belongs to a captured service",
                ));
            }
            repo.put(
                &format!("catalog/external-services/{}.json", value.id),
                &value,
                args.expected_input_digest.as_deref(),
            )
        }
        Command::List(args) => {
            let repo = Repository::open(&args.root)?;
            let rows = records(&repo)?;
            super::cli::page(
                &digest(&rows)?,
                rows.into_values().map(|s| json!(s)).collect(),
                args.cursor.as_deref(),
                args.limit as usize,
                json!({"inputDigest":repo.input_digest()?,"authority":"DECLARED_EXTERNAL_SERVICE"}),
            )
        }
        Command::Show { root, id } => {
            if !store::valid_id(&id) {
                return Err(invalid("invalid external-service ID"));
            }
            let repo = Repository::open(&root)?;
            Ok(
                json!({"record":records(&repo)?.get(&id).ok_or_else(||invalid("external service not registered"))?,"authority":"DECLARED_EXTERNAL_SERVICE","sourceCapture":"NOT_CAPTURED","inputDigest":repo.input_digest()?}),
            )
        }
        Command::Remove(args) => super::removal::run("external-service", args),
    }
}

pub(super) fn for_page(checked: &Check, subject: &str) -> Value {
    let inputs = checked.source_inputs.as_ref().map(|s| &s.inputs);
    let Some(inputs) = inputs else {
        return json!({});
    };
    let selected: Vec<_> = inputs
        .interactions
        .values()
        .filter(|i| {
            i.external_service.is_some()
                && (subject
                    .strip_prefix("service:")
                    .is_some_and(|s| i.from.service == s)
                    || subject
                        .strip_prefix("scenario:")
                        .and_then(|id| inputs.scenarios.get(id))
                        .is_some_and(|s| s.interactions.contains(&i.id)))
        })
        .filter_map(|i| i.external_service.as_ref())
        .collect();
    json!(
        inputs
            .external_services
            .iter()
            .filter(|(id, _)| selected.contains(id))
            .collect::<BTreeMap<_, _>>()
    )
}

pub(super) fn page_links(data: &Value) -> String {
    let rows=data["externalServices"].as_object().into_iter().flatten().map(|(id,value)|format!("<li><a href=\"../external-services/{}.html\">{}</a> — declared information; source not captured</li>",escape(id),escape(value["title"].as_str().unwrap_or(id)))).collect::<String>();
    if rows.is_empty() {
        String::new()
    } else {
        format!(
            "<section aria-label=\"External service information\"><h2>External services</h2><ul>{rows}</ul></section>"
        )
    }
}

pub(super) fn publication(
    checked: &Check,
    files: &mut BTreeMap<String, Vec<u8>>,
    cards: &mut String,
) -> Result<(), ClewError> {
    let Some(inputs) = &checked.source_inputs else {
        return Ok(());
    };
    for (id, value) in &inputs.inputs.external_services {
        validate(value)?;
        let mut links = String::new();
        let mut md_links = String::new();
        for (label, url) in [("Repository", &value.repository), ("Wiki", &value.wiki)] {
            if let Some(url) = url {
                links += &format!(
                    "<li><a href=\"{}\" rel=\"noreferrer noopener\">{label}</a></li>",
                    escape(url)
                );
                md_links += &format!("- [{label}]({url})\n");
            }
        }
        let related=inputs.inputs.interactions.values().filter(|i|i.external_service.as_deref()==Some(id)).map(|i|format!("<li>{}: <a href=\"../services/{}.html\">{}</a> → {} (declared interaction; runtime unverified)</li>",escape(&i.title),escape(&i.from.service),escape(&i.from.service),escape(&i.to.service))).collect::<String>();
        let limitations = value
            .limitations
            .iter()
            .map(|s| format!("<li>{}</li>", escape(s)))
            .collect::<String>();
        let html = format!(
            "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><meta name=\"referrer\" content=\"no-referrer\"><title>{}</title><style>body{{max-width:55rem;margin:3rem auto;padding:0 1rem;font:18px/1.6 system-ui}}p{{white-space:pre-wrap}}</style></head><body><nav><a href=\"../overview.html\">Documentation overview</a></nav><main><p>DECLARED EXTERNAL SERVICE · SOURCE NOT CAPTURED · RUNTIME UNVERIFIED</p><h1>{}</h1><p>{}</p><ul>{links}</ul><h2>Declared interactions</h2><ul>{related}</ul><h2>Limitations</h2><ul>{limitations}</ul><details><summary>Declaration provenance</summary><pre>{}</pre></details></main></body></html>",
            escape(&value.title),
            escape(&value.title),
            escape(&value.description),
            escape(&serde_json::to_string_pretty(&value.declaration).map_err(io_error)?)
        );
        let metadata=serde_json::to_string(&json!({"title":value.title,"externalService":value,"authority":"DECLARED_EXTERNAL_SERVICE","sourceCapture":"NOT_CAPTURED"})).map_err(io_error)?.replace('<',"\\u003c");
        let html = html.replace(
            "</body>",
            &format!(
                "<script type=\"application/json\" id=\"document-data\">{metadata}</script></body>"
            ),
        );
        files.insert(format!("external-services/{id}.html"), html.into_bytes());
        files.insert(format!("external-services/{id}.json"),bytes(&json!({"externalService":value,"authority":"DECLARED_EXTERNAL_SERVICE","sourceCapture":"NOT_CAPTURED","runtime":"UNVERIFIED"}))?);
        files.insert(format!("external-services/{id}.md"),format!("# {}\n\nDeclared external service. Source not captured; runtime unverified.\n\n{}\n\n{md_links}",value.title,value.description).into_bytes());
        *cards += &format!(
            "<article class=\"gap-card\"><p>External service · declared information</p><h2><a href=\"generated/__BUNDLE__/external-services/{id}.html\">{}</a></h2><p>{}</p></article>",
            escape(&value.title),
            escape(&value.description)
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn record() -> ExternalService {
        serde_json::from_value(json!({"schema":SCHEMA,"id":"forecast-provider","title":"Forecast Provider","description":"Declared forecast provider","repository":"https://github.com/owned/provider","wiki":"https://docs.example.invalid/forecast","declaration":{"origin":"human","rationale":"Owned fixture"},"limitations":["Target code not captured"]})).unwrap()
    }
    #[test]
    fn invalid_links_and_colliding_service_identity_are_refused() {
        for link in [
            "javascript:alert(1)",
            "http://example.invalid/docs",
            "https://user@example.invalid/docs",
            "https://example.invalid/docs?secret=1",
            "https://example.invalid/../private",
        ] {
            let mut r = record();
            r.wiki = Some(link.into());
            assert!(validate(&r).is_err(), "{link}");
        }
        let t = tempfile::tempdir().unwrap();
        Repository::init(t.path(), "External fixture").unwrap();
        let repo = Repository::open(t.path()).unwrap();
        let value = record();
        repo.put(
            "catalog/external-services/forecast-provider.json",
            &value,
            Some(&repo.input_digest().unwrap()),
        )
        .unwrap();
        let service:super::super::model::Service=serde_json::from_value(json!({"schema":"codeclew-documentation-service/1.0","id":value.id,"title":"Captured","repositoryId":"owned","repository":"https://github.com/owned/service","language":"java","profile":"source-syntax","targetRef":"main","source":{"roots":["src"],"dialect":"17"}})).unwrap();
        assert!(
            repo.service_add(service, Some(&repo.input_digest().unwrap()))
                .is_err()
        );
    }
    #[test]
    fn registered_interaction_blocks_external_removal_and_missing_target_is_explicit() {
        let t = tempfile::tempdir().unwrap();
        Repository::init(t.path(), "External fixture").unwrap();
        let repo = Repository::open(t.path()).unwrap();
        let service:super::super::model::Service=serde_json::from_value(json!({"schema":"codeclew-documentation-service/1.0","id":"weather","title":"Weather","repositoryId":"owned","repository":"https://github.com/owned/service","language":"java","profile":"source-syntax","targetRef":"main","source":{"roots":["src"],"dialect":"17"}})).unwrap();
        repo.service_add(service, Some(&repo.input_digest().unwrap()))
            .unwrap();
        let interaction:super::super::model::Interaction=serde_json::from_value(json!({"schema":"codeclew-documentation-interaction/1.0","id":"fetch","title":"Fetch forecast","from":{"service":"weather"},"to":{"service":"forecast-provider"},"external":true,"externalService":"forecast-provider","transport":{"kind":"http","method":"GET","path":"/forecast"},"declaration":{"origin":"human","rationale":"Owned boundary"}})).unwrap();
        assert!(
            repo.interaction_put(interaction.clone(), Some(&repo.input_digest().unwrap()))
                .is_err()
        );
        repo.put(
            "catalog/external-services/forecast-provider.json",
            &record(),
            Some(&repo.input_digest().unwrap()),
        )
        .unwrap();
        repo.interaction_put(interaction, Some(&repo.input_digest().unwrap()))
            .unwrap();
        let request = |apply| super::super::removal::Request {
            root: t.path().into(),
            id: "forecast-provider".into(),
            expected_input_digest: Some(repo.input_digest().unwrap()),
            apply,
        };
        let impact = super::super::removal::run("external-service", request(false)).unwrap();
        assert_eq!(impact["status"], "BLOCKED");
        assert!(super::super::removal::run("external-service", request(true)).is_err());
        repo.interaction_remove("fetch", &repo.input_digest().unwrap())
            .unwrap();
        assert_eq!(
            super::super::removal::run("external-service", request(true)).unwrap()["status"],
            "REMOVED"
        );
        assert!(
            serde_json::to_value(repo.inputs().unwrap())
                .unwrap()
                .get("externalServices")
                .is_none()
        );
    }
}
