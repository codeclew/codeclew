//! Declared domain identities never inferred from class names or renamed candidates.
use super::{
    bytes,
    check::Check,
    cli::{InputArgs, ListArgs},
    digest, invalid,
    model::Observation,
    store::{self, Repository},
};
use crate::error::{ClewError, ErrorCode};
use clap::Subcommand;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Entity {
    pub schema: String,
    pub id: String,
    pub title: String,
    pub description: String,
    pub relations: Vec<Relation>,
    #[serde(default)]
    pub related_entities: Vec<String>,
    pub limitations: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Relation {
    pub service: String,
    pub kind: String,
    pub origin: String,
    pub rationale: String,
    pub confidence: String,
    #[serde(default)]
    pub representations: Vec<String>,
    #[serde(default)]
    pub dependency_ids: Vec<String>,
}
pub fn validate(entity: &Entity, services: &BTreeSet<String>) -> Result<(), ClewError> {
    if entity.schema != "codeclew-documentation-entity/1.0"
        || !store::valid_id(&entity.id)
        || entity.title.trim().is_empty()
        || entity.title.len() > 512
        || entity.description.len() > 8192
        || entity.relations.len() > 128
        || entity.related_entities.len() > 64
        || entity.limitations.len() > 64
    {
        return Err(invalid("invalid domain entity identity or budget"));
    }
    for r in &entity.relations {
        if !services.contains(&r.service)
            || !matches!(
                r.kind.as_str(),
                "created" | "changed" | "read" | "stored-copy" | "owned"
            )
            || !matches!(r.origin.as_str(), "human" | "agent-proposal")
            || !matches!(
                r.confidence.as_str(),
                "declared" | "supported" | "uncertain"
            )
            || r.rationale.trim().is_empty()
            || r.rationale.len() > 4096
            || r.representations.len() > 64
            || r.dependency_ids.len() > 128
        {
            return Err(invalid(
                "invalid entity relationship, provenance or confidence",
            ));
        }
    }
    if entity
        .related_entities
        .iter()
        .any(|id| !store::valid_id(id) || id == &entity.id)
    {
        return Err(invalid("entity links require explicit distinct domain IDs"));
    }
    Ok(())
}
pub fn records(repo: &Repository) -> Result<BTreeMap<String, Entity>, ClewError> {
    let entities: BTreeMap<String, Entity> = repo.records("catalog/entities", "json")?;
    let services = repo.services()?.into_keys().collect();
    for (id, entity) in &entities {
        validate(entity, &services)?;
        if id != &entity.id
            || entity
                .related_entities
                .iter()
                .any(|id| !entities.contains_key(id))
        {
            return Err(invalid(
                "entity filename mismatch or dangling explicit entity link",
            ));
        }
    }
    Ok(entities)
}
pub fn put(
    repo: &Repository,
    entity: Entity,
    expected: Option<&str>,
    human: bool,
) -> Result<Value, ClewError> {
    let _lock = repo.lock()?;
    let current = repo.input_digest()?;
    if expected != Some(current.as_str()) {
        return Err(ClewError::new(
            ErrorCode::WwConflict,
            "entity catalogue changed; reload inputDigest",
        ));
    }
    validate(&entity, &repo.services()?.into_keys().collect())?;
    let old = records(repo)?;
    if entity
        .related_entities
        .iter()
        .any(|id| !old.contains_key(id))
    {
        return Err(invalid(
            "unknown related entity; register its stable identity first",
        ));
    }
    if !human {
        let protected = |e: &Entity| {
            e.relations
                .iter()
                .filter(|r| r.origin == "human")
                .cloned()
                .collect::<Vec<_>>()
        };
        if protected(&entity) != old.get(&entity.id).map(protected).unwrap_or_default() {
            return Err(invalid(
                "agent proposals cannot add, replace or remove human entity declarations",
            ));
        }
    }
    repo.atomic(
        &format!("catalog/entities/{}.json", entity.id),
        &bytes(&entity)?,
    )?;
    Ok(
        json!({"status":"SAVED","inputDigest":repo.input_digest()?,"ownershipAuthority":"Human declarations remain distinct from agent proposals"}),
    )
}
pub fn attach(repo: &Repository, checked: &mut Check) -> Result<(), ClewError> {
    let rows = records(repo)?;
    for (id, entity) in &rows {
        let key = format!("entity:{id}");
        let deps: BTreeSet<_> = entity
            .relations
            .iter()
            .flat_map(|r| r.dependency_ids.iter().cloned())
            .chain(
                entity
                    .related_entities
                    .iter()
                    .map(|id| format!("entity:{id}")),
            )
            .collect();
        let missing: Vec<_> = deps
            .iter()
            .filter(|id| !id.starts_with("entity:") && !checked.dependencies.contains_key(*id))
            .cloned()
            .collect();
        let sources: BTreeSet<_> = deps
            .iter()
            .filter_map(|id| checked.dependencies.get(id))
            .flat_map(|d| d.source_ids.iter().cloned())
            .collect();
        let normalized = json!({"entity":entity,"dependencyIds":deps,"missingDependencies":missing,"authority":"DECLARED_DOMAIN_ENTITY","ownership":"Human declarations and agent proposals are separate; no runtime ownership proof"});
        checked.dependencies.insert(
            key.clone(),
            Observation {
                id: key,
                kind: "DOMAIN_ENTITY".into(),
                service: String::new(),
                symbol: entity.title.clone(),
                digest: digest(&normalized)?,
                normalized,
                source_ids: sources.into_iter().collect(),
            },
        );
    }
    for service in checked.services.keys() {
        let members: Vec<_> = rows
            .values()
            .filter(|e| e.relations.iter().any(|r| &r.service == service))
            .map(|e| format!("entity:{}", e.id))
            .collect();
        let normalized =
            json!({"dependencyIds":members,"authority":"DECLARED_DOMAIN_ENTITY_SCOPE"});
        let key = format!("entity-scope:{service}");
        checked.dependencies.insert(
            key.clone(),
            Observation {
                id: key,
                kind: "ENTITY_SCOPE".into(),
                service: service.clone(),
                symbol: service.clone(),
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
    List(ListArgs),
    Put {
        #[command(flatten)]
        input: InputArgs,
        #[arg(long)]
        human: bool,
    },
}
pub fn run(command: Command) -> Result<Value, ClewError> {
    match command {
        Command::List(page) => {
            let repo = Repository::open(&page.root)?;
            let rows = records(&repo)?;
            super::cli::page(
                &digest(&rows)?,
                rows.into_values().map(|e| json!(e)).collect(),
                page.cursor.as_deref(),
                page.limit as usize,
                json!({"inputDigest":repo.input_digest()?}),
            )
        }
        Command::Put { input, human } => put(
            &Repository::open(&input.root)?,
            store::read(&input.input, store::MAX_RECORD)?,
            input.expected_input_digest.as_deref(),
            human,
        ),
    }
}
