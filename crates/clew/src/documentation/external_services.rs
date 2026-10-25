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

mod configuration;
pub use configuration::ConfigurationReference;

pub(in crate::documentation) fn configuration_source_label(state: &str) -> &'static str {
    configuration::source_label(state)
}
pub(in crate::documentation) fn configuration_reference_label(kind: &str) -> &'static str {
    configuration::reference_label(kind)
}

pub(super) const SCHEMA: &str = "codeclew-documentation-external-service/1.0";
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExternalService {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub configuration_references: Vec<ConfigurationReference>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub identity: Option<IdentityProfile>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub references: Vec<KnowledgeReference>,
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
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IdentityProfile {
    pub group: String,
    pub title: String,
    pub profile: String,
    pub status: String,
    pub declaration: super::model::Declaration,
    #[serde(default)]
    pub dependency_ids: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct KnowledgeReference {
    pub id: String,
    pub title: String,
    pub kind: String,
    pub location: String,
    pub status: String,
    pub declaration: super::model::Declaration,
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
    if value.category.as_deref().is_some_and(|category| {
        !matches!(
            category,
            "system" | "storage" | "queue" | "configuration" | "unknown"
        )
    }) || value.references.len() > 64
    {
        return Err(invalid("invalid destination category or reference bound"));
    }
    let declaration = |d: &super::model::Declaration| {
        matches!(d.origin.as_str(), "human" | "agent-proposal" | "imported")
            && !d.rationale.trim().is_empty()
            && d.rationale.len() <= 4096
    };
    if let Some(identity) = &value.identity
        && (!store::valid_id(&identity.group)
            || identity.title.trim().is_empty()
            || identity.title.len() > 512
            || identity.profile.trim().is_empty()
            || identity.profile.len() > 512
            || !matches!(
                identity.status.as_str(),
                "proposed" | "confirmed" | "rejected"
            )
            || !declaration(&identity.declaration)
            || identity.dependency_ids.len() > 128
            || (identity.status == "confirmed" && identity.declaration.origin != "human"))
    {
        return Err(invalid(
            "identity needs a bounded explicit hypothesis or human confirmation",
        ));
    }
    let mut ids = std::collections::BTreeSet::new();
    for reference in &value.references {
        if !store::valid_id(&reference.id)
            || !ids.insert(&reference.id)
            || reference.title.trim().is_empty()
            || reference.title.len() > 512
            || !matches!(reference.kind.as_str(), "wiki" | "document" | "code")
            || !matches!(
                reference.status.as_str(),
                "unverified" | "reviewed" | "stale"
            )
            || !declaration(&reference.declaration)
            || reference.location.len() > 2048
            || super::source_links::web_base(&reference.location).is_err()
        {
            return Err(invalid(
                "invalid supplemental reference; use a clean HTTPS location and explicit provenance/status",
            ));
        }
    }
    configuration::validate(&value.configuration_references)?;
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
    // Historical input manifests without the registry keep their existing
    // declaration-reading contract, including partial source catalogues.
    // Live process writes always validate captured and external participants.
    if !inputs.external_services.is_empty() {
        let external = inputs.external_services.keys().cloned().collect();
        for scenario in inputs.scenarios.values() {
            super::processes::validate_with_external(scenario, &inputs.services, &external)?;
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
        let normalized = json!({"externalService":value,"configurationEvidence":configuration::project(checked,&value.configuration_references),"interactions":inputs.interactions.values().filter(|i| i.external_service.as_deref()==Some(id)).collect::<Vec<_>>(),"authority":"DECLARED_EXTERNAL_SERVICE","sourceCapture":"NOT_CAPTURED","runtime":"UNVERIFIED"});
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
    Put {
        #[command(flatten)]
        input: InputArgs,
        #[arg(long)]
        human: bool,
    },
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
        Command::Put { input: args, human } => {
            let repo = Repository::open(&args.root)?;
            let value: ExternalService = store::read(&args.input, 64 * 1024)?;
            validate(&value)?;
            let previous = records(&repo)?.remove(&value.id);
            if !human
                && (value
                    .identity
                    .as_ref()
                    .is_some_and(|i| i.status == "confirmed")
                    || previous
                        .as_ref()
                        .and_then(|v| v.identity.as_ref())
                        .is_some_and(|i| i.declaration.origin == "human"))
                && previous.as_ref().and_then(|v| v.identity.as_ref()) != value.identity.as_ref()
            {
                return Err(invalid(
                    "human identity decisions require --human; agent proposals cannot replace them",
                ));
            }
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

pub(super) fn declaration_inputs(checked: &Check) -> Option<&super::store::RepositoryInputs> {
    checked
        .composition
        .as_ref()
        .map(|c| &c.inputs)
        .or_else(|| checked.source_inputs.as_ref().map(|s| &s.inputs))
}

pub(super) fn for_page(checked: &Check, subject: &str) -> Value {
    let inputs = declaration_inputs(checked);
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
    let mut selected = selected;
    if let Some(process) = subject
        .strip_prefix("scenario:")
        .and_then(|id| inputs.scenarios.get(id))
        .and_then(|s| s.process.as_ref())
    {
        selected.extend(
            process
                .participants
                .iter()
                .filter(|id| inputs.external_services.contains_key(*id)),
        );
    }
    json!(
        inputs
            .external_services
            .iter()
            .filter(|(id, _)| selected.contains(id))
            .collect::<BTreeMap<_, _>>()
    )
}

fn profile_html(
    checked: &Check,
    id: &str,
    value: &ExternalService,
    inputs: &super::store::RepositoryInputs,
    language: &str,
) -> String {
    let l = |key| super::language::label(language, key);
    let mut body = format!(
        "<nav><a href=\"../overview.html\">{}</a></nav><h1>{}</h1><p>{}</p><p>{}</p>",
        l("Documentation overview"),
        escape(&value.title),
        escape(&value.description),
        l("Declared information; source not captured. Runtime unverified.")
    );
    if let Some(identity) = &value.identity {
        let status = match identity.status.as_str() {
            "confirmed" => l("Human-confirmed grouping"),
            "rejected" => l("Grouping rejected; profiles remain distinct"),
            _ => l("Identity hypothesis"),
        };
        body += &format!(
            "<section><h2>{status}</h2><p>{}</p><p>{}</p><p><b>{}</b>: {}</p>",
            escape(&identity.title),
            escape(&identity.declaration.rationale),
            l("Profile"),
            escape(&identity.profile)
        );
        if identity.status != "rejected" {
            body += "<ul>";
            for (peer_id, peer) in &inputs.external_services {
                if peer
                    .identity
                    .as_ref()
                    .is_some_and(|p| p.group == identity.group && p.status != "rejected")
                {
                    body += &format!(
                        "<li><a href=\"{}.html\">{}</a> — {}</li>",
                        escape(peer_id),
                        escape(&peer.title),
                        escape(&peer.identity.as_ref().unwrap().profile)
                    );
                }
            }
            body += "</ul>";
        }
        body += &format!(
            "<details><summary>{}</summary><pre>{}</pre><p>{}</p><code>clew docs external-service put --root &lt;root&gt; --input &lt;profile.json&gt; --expected-input-digest &lt;digest&gt;</code><p>{}</p></details></section>",
            l("Review or edit identity decision"),
            escape(&serde_json::to_string_pretty(identity).unwrap_or_default()),
            l("Keep profile IDs and evidence. Confirmed human decisions require --human."),
            l("References and grouping never prove physical service identity or runtime routing.")
        );
    }
    body += &configuration::html(
        &configuration::project(checked, &value.configuration_references),
        language,
    );
    body += &format!("<section><h2>{}</h2>", l("Operations and configuration"));
    let mut groups: BTreeMap<&str, Vec<&super::model::Interaction>> = BTreeMap::new();
    for interaction in inputs
        .interactions
        .values()
        .filter(|i| i.external_service.as_deref() == Some(id))
    {
        groups
            .entry(&interaction.from.service)
            .or_default()
            .push(interaction);
    }
    for (service, operations) in groups {
        let service_title = inputs
            .services
            .get(service)
            .map(|s| s.title.as_str())
            .unwrap_or(service);
        body += &format!(
            "<section class=\"profile-service\"><h3><a href=\"../services/{}.html#section-egress\">{}</a></h3>",
            escape(service),
            escape(service_title)
        );
        let mut binding_groups: BTreeMap<Option<&str>, Vec<&super::model::Interaction>> =
            BTreeMap::new();
        for operation in operations {
            binding_groups
                .entry(operation.transport.destination_config_key.as_deref())
                .or_default()
                .push(operation);
        }
        for operations in binding_groups.into_values() {
            let common_key = operations
                .first()
                .and_then(|i| i.transport.destination_config_key.as_deref())
                .filter(|key| {
                    operations
                        .iter()
                        .all(|i| i.transport.destination_config_key.as_deref() == Some(key))
                });
            if let Some(key) = common_key {
                body += &format!(
                    "<p>{}: <code>{}</code></p>",
                    l("Shared configuration key"),
                    escape(key)
                );
            }
            body += "<ul class=\"profile-operations\">";
            for interaction in operations {
                let transport = &interaction.transport;
                let destination = match transport.kind.as_str() {
                    "http" => format!(
                        "{} {}",
                        transport.method.as_deref().unwrap_or("HTTP"),
                        transport.path.as_deref().unwrap_or("")
                    ),
                    "kafka" => format!(
                        "{} {}",
                        l("Topic"),
                        transport.topic.as_deref().unwrap_or("")
                    ),
                    _ => l("Logical access").into(),
                };
                body += &format!(
                    "<li><details><summary><b>{}</b> · <code>{}</code></summary>",
                    escape(&interaction.title),
                    escape(&destination)
                );
                if common_key.is_none()
                    && let Some(key) = &transport.destination_config_key
                {
                    body += &format!(
                        "<p>{}: <code>{}</code></p>",
                        l("Configuration key"),
                        escape(key)
                    );
                }
                if transport.kind == "kafka" {
                    body += &format!(
                        "<p>{}: <code>{}</code></p><p>{}</p>",
                        l("Message key"),
                        escape(
                            transport
                                .message_key
                                .as_deref()
                                .unwrap_or(l("Not established"))
                        ),
                        l(
                            "A topic name does not establish its payload schema or consumer contract."
                        )
                    );
                }
                if let Some(purpose) = &transport.purpose {
                    body += &format!("<p>{}</p>", escape(purpose));
                }
                for address in &interaction.addresses {
                    body += &format!(
                        "<p><code>{}</code> · {} · {}</p>",
                        escape(&address.url),
                        escape(&address.source),
                        escape(address.environment.as_deref().unwrap_or(""))
                    );
                }
                let processes = inputs
                    .scenarios
                    .values()
                    .filter(|p| p.interactions.contains(&interaction.id))
                    .collect::<Vec<_>>();
                if !processes.is_empty() {
                    body += &format!("<p>{}: ", l("Related processes and contracts"));
                    for process in processes {
                        body += &format!(
                            "<a href=\"../scenarios/{}.html\">{}</a> ",
                            escape(&process.id),
                            escape(&process.title)
                        );
                    }
                    body += "</p>";
                }
                body += &format!(
                    "<details><summary>{}</summary><pre>{}</pre></details></details></li>",
                    l("Declaration evidence"),
                    escape(&serde_json::to_string_pretty(interaction).unwrap_or_default())
                );
            }
            body += "</ul>";
        }
        body += "</section>";
    }
    body += "</section>";
    body += &format!(
        "<section><h2>{}</h2><p>{}</p><ul>",
        l("Associated knowledge"),
        l("Supplemental references are separate from captured implementation.")
    );
    for (title, url) in [
        (l("Repository"), &value.repository),
        (l("Wiki"), &value.wiki),
    ] {
        if let Some(url) = url {
            body += &format!(
                "<li><a href=\"{}\" rel=\"noreferrer noopener\">{}</a> · {}</li>",
                escape(url),
                title,
                l("Legacy reference; unverified")
            );
        }
    }
    for reference in &value.references {
        body += &format!(
            "<li><a href=\"{}\" rel=\"noreferrer noopener\">{}</a> · {} · {}<details><summary>{}</summary><p>{}: {}</p></details></li>",
            escape(&reference.location),
            escape(&reference.title),
            escape(&reference.kind),
            escape(&reference.status),
            l("Provenance"),
            escape(&reference.declaration.origin),
            escape(&reference.declaration.rationale)
        );
    }
    body += &format!(
        "</ul></section><details><summary>{}</summary><ul>{}</ul><pre>{}</pre></details>",
        l("Limitations and provenance"),
        value
            .limitations
            .iter()
            .map(|s| format!("<li>{}</li>", escape(s)))
            .collect::<String>(),
        escape(&serde_json::to_string_pretty(&value.declaration).unwrap_or_default())
    );
    super::reader::page_language(&value.title, &body, language)
}

pub(super) fn publication(
    checked: &Check,
    files: &mut BTreeMap<String, Vec<u8>>,
    cards: &mut String,
    language: &str,
) -> Result<(), ClewError> {
    let Some(inputs) = declaration_inputs(checked) else {
        return Ok(());
    };
    for (id, value) in &inputs.external_services {
        validate(value)?;
        let html = profile_html(checked, id, value, inputs, language);
        let mut md_links = value
            .references
            .iter()
            .map(|r| {
                format!(
                    "- [{}]({}) · {} · {}\n",
                    r.title, r.location, r.kind, r.status
                )
            })
            .collect::<String>();
        for (title, url) in [("Repository", &value.repository), ("Wiki", &value.wiki)] {
            if let Some(url) = url {
                md_links += &format!(
                    "- [{title}]({})\n",
                    url.replace('(', "%28").replace(')', "%29")
                );
            }
        }
        let metadata=serde_json::to_string(&json!({"title":value.title,"externalService":value,"configurationEvidence":configuration::project(checked,&value.configuration_references),"interactions":inputs.interactions.values().filter(|i| i.external_service.as_deref()==Some(id)).collect::<Vec<_>>(),"authority":"DECLARED_EXTERNAL_SERVICE","sourceCapture":"NOT_CAPTURED"})).map_err(io_error)?.replace('<',"\\u003c");
        let html = html.replace(
            "</body>",
            &format!(
                "<script type=\"application/json\" id=\"document-data\">{metadata}</script></body>"
            ),
        );
        files.insert(format!("external-services/{id}.html"), html.into_bytes());
        files.insert(format!("external-services/{id}.json"),bytes(&json!({"externalService":value,"configurationEvidence":configuration::project(checked,&value.configuration_references),"interactions":inputs.interactions.values().filter(|i| i.external_service.as_deref()==Some(id)).collect::<Vec<_>>(),"authority":"DECLARED_EXTERNAL_SERVICE","sourceCapture":"NOT_CAPTURED","runtime":"UNVERIFIED"}))?);
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
    fn native_profile_groups_operations_by_service_and_binding_with_transport_specific_labels() {
        let tmp = tempfile::tempdir().unwrap();
        Repository::init(tmp.path(), "Grouped profile fixture").unwrap();
        let mut inputs = Repository::open(tmp.path()).unwrap().inputs().unwrap();
        for (id, transport) in [
            (
                "quote",
                json!({"kind":"http","method":"POST","path":"/quote","destinationConfigKey":"pricing.endpoint"}),
            ),
            (
                "renew",
                json!({"kind":"http","method":"POST","path":"/renew","destinationConfigKey":"pricing.endpoint"}),
            ),
            (
                "alternate",
                json!({"kind":"http","method":"POST","path":"/quote","destinationConfigKey":"pricing.alternate"}),
            ),
            ("store", json!({"kind":"logical"})),
            (
                "publish",
                json!({"kind":"kafka","topic":"observation.created"}),
            ),
        ] {
            inputs.interactions.insert(id.into(),serde_json::from_value(json!({"schema":"codeclew-documentation-interaction/1.0","id":id,"title":id,"from":{"service":"owner"},"to":{"service":"forecast-provider"},"external":true,"externalService":"forecast-provider","transport":transport,"declaration":{"origin":"human","rationale":"Explicit fixture boundary"}})).unwrap());
        }
        let checked:Check=serde_json::from_value(json!({"schema":"fixture","inputDigest":"input","contextDigest":"context","services":{},"unresolved":{},"interactions":{},"scenarios":{},"dependencies":{}})).unwrap();
        let before = serde_json::to_value(&inputs).unwrap();
        let html = profile_html(&checked, "forecast-provider", &record(), &inputs, "en");
        assert_eq!(
            html.matches("../services/owner.html#section-egress")
                .count(),
            1
        );
        assert_eq!(
            html.matches("Shared configuration key: <code>pricing.endpoint</code>")
                .count(),
            1
        );
        for title in ["quote", "renew", "alternate", "store", "publish"] {
            assert!(html.contains(title));
        }
        assert!(html.contains("Logical access"));
        assert!(html.contains("Topic observation.created"));
        assert!(!html.contains("Concrete URL"));
        assert_eq!(serde_json::to_value(inputs).unwrap(), before);
    }
    #[test]
    fn hypotheses_references_and_legacy_serialization_keep_distinct_authority() {
        let old = record();
        let serialized = serde_json::to_value(&old).unwrap();
        assert!(
            serialized.get("category").is_none()
                && serialized.get("identity").is_none()
                && serialized.get("references").is_none()
        );
        let mut value = serialized;
        value["category"] = json!("system");
        value["identity"] = json!({"group":"provider","title":"Provider","profile":"primary configuration","status":"proposed","declaration":{"origin":"agent-proposal","rationale":"Owned synthetic hypothesis"},"dependencyIds":[]});
        value["references"] = json!([{"id":"docs","title":"Documentation","kind":"document","location":"https://docs.example.invalid/provider","status":"unverified","declaration":{"origin":"human","rationale":"Supplemental only"}}]);
        let external: ExternalService = serde_json::from_value(value.clone()).unwrap();
        validate(&external).unwrap();
        assert_eq!(external.declaration, old.declaration);
        // Build the synthetic negative case with the required Python standard
        // library, keeping fixture credentials explicit without adding a URL
        // crate solely for this test. Nothing is resolved or sent over a network.
        let password_url = std::process::Command::new("python3")
            .args(["-I", "-S", "-c", r#"
import json
from urllib.parse import quote, urlsplit, urlunsplit
endpoint = urlsplit("https://example.invalid/docs")
fixture_username = "fixture-user"
fixture_password = "fixture-password"
authority = quote(fixture_username, safe="") + ":" + quote(fixture_password, safe="") + "@" + endpoint.netloc
location = urlunsplit(endpoint._replace(netloc=authority))
parsed = urlsplit(location)
assert parsed.hostname == "example.invalid"
print(json.dumps({"location": location, "passwordPresent": parsed.password == fixture_password and bool(parsed.password)}))
"#])
            .output()
            .unwrap();
        assert!(
            password_url.status.success(),
            "{}",
            String::from_utf8_lossy(&password_url.stderr)
        );
        let password_url: Value = serde_json::from_slice(&password_url.stdout).unwrap();
        assert_eq!(password_url["passwordPresent"], true);
        for (pointer, replacement) in [
            ("/identity/status", json!("confirmed")),
            ("/references/0/location", password_url["location"].clone()),
            ("/references/0/status", json!("SOURCE_PROVEN")),
            ("/category", json!("inferred-physical-database")),
        ] {
            let mut bad = value.clone();
            *bad.pointer_mut(pointer).unwrap() = replacement;
            assert!(
                validate(&serde_json::from_value(bad).unwrap()).is_err(),
                "{pointer}"
            );
        }
        let mut rejected = external.clone();
        rejected.identity.as_mut().unwrap().status = "rejected".into();
        validate(&rejected).unwrap();
        assert_eq!(rejected.id, external.id);
        assert_eq!(
            rejected.identity.as_ref().unwrap().profile,
            external.identity.as_ref().unwrap().profile
        );
    }

    #[test]
    fn process_participants_accept_registered_external_actors_but_never_external_roots() {
        let service:super::super::model::Service=serde_json::from_value(json!({"schema":"codeclew-documentation-service/1.0","id":"weather","title":"Weather","repositoryId":"owned","repository":"https://github.com/owned/weather.git","language":"java","profile":"source-syntax","targetRef":"main","source":{"roots":["src"],"dialect":"17"}})).unwrap();
        let services = BTreeMap::from([("weather".into(), service)]);
        let external = std::collections::BTreeSet::from(["forecast-provider".into()]);
        let mut process:super::super::model::Scenario=serde_json::from_value(json!({"schema":"codeclew-documentation-process/1.0","id":"forecast","title":"Forecast","summary":"Owned process","root":{"service":"weather"},"process":{"scope":"Declared boundary","participants":["weather","forecast-provider"],"trigger":"Request","outcomes":["Static forecast"]}})).unwrap();
        super::super::processes::validate_with_external(&process, &services, &external).unwrap();
        process
            .process
            .as_mut()
            .unwrap()
            .participants
            .push("missing-provider".into());
        assert!(
            super::super::processes::validate_with_external(&process, &services, &external)
                .is_err()
        );
        process.process.as_mut().unwrap().participants.pop();
        process.root.service = "forecast-provider".into();
        assert!(
            super::super::processes::validate_with_external(&process, &services, &external)
                .is_err()
        );
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
