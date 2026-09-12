//! Stable document roots, separate from discovered source callable identities.
use super::{bindings, check::Check, cli::ListArgs, invalid, model::*, store::Repository, work};
use crate::error::ClewError;
use clap::Subcommand;
use serde_json::{Value, json};
use std::{collections::BTreeSet, path::PathBuf};

pub const REQUIRED: [(&str, &str, &str); 5] = [
    (
        "section-overview",
        "Overview",
        "Describe the service purpose, scope and supported outcomes.",
    ),
    (
        "section-responsibilities",
        "Responsibilities",
        "Describe responsibilities and exclusions from evidence; business intent may need a human note.",
    ),
    (
        "section-entities",
        "Domain entities",
        "Identify domain concepts separately from DTOs and storage classes; ownership needs an explicit declaration.",
    ),
    (
        "section-ingress",
        "Ingress contracts",
        "Document discovered public boundaries and declared contracts; retain gaps for unsupported discovery.",
    ),
    (
        "section-egress",
        "Egress contracts",
        "Document external calls and messages; unresolved destinations and runtime activation remain gaps.",
    ),
];

pub fn contains(id: &str) -> bool {
    REQUIRED.iter().any(|(key, _, _)| *key == id)
}
pub fn ids() -> impl Iterator<Item = String> {
    REQUIRED.iter().map(|(id, _, _)| (*id).into())
}
pub fn expected(evidence: &ServiceEvidence) -> BTreeSet<String> {
    evidence
        .entrypoints
        .iter()
        .map(|e| e.id.clone())
        .chain(ids())
        .collect()
}
pub fn records(service: &str, narrative: Option<&Narrative>) -> Vec<Value> {
    REQUIRED.iter().map(|(id, title, purpose)| {
        let content = narrative.and_then(|n| n.operations.iter().find(|o| o.id == *id));
        json!({"schema":"codeclew-documentation-section/1.0","id":id,
            "objectId":format!("service:{service}/{id}"),"service":service,"title":title,
            "purpose":purpose,"required":true,"status":if content.is_some(){"AUTHORED"}else{"GAP"},
            "content":content,"gap":narrative.and_then(|n|n.gaps.get(*id)).cloned().unwrap_or_else(||format!("{purpose} Source-bound content has not been accepted yet.")),
            "workRequest":{"schema":"codeclew-documentation-work-request/1.0","audience":"Service maintainers and architecture readers","entrypoint":id,"maxItems":20,"maxBytes":40960}})
    }).collect()
}

pub fn inventory(service: &str, checked: &Check) -> Value {
    let Some(evidence) = checked.services.get(service) else {
        return json!({"publicBoundaries":[],"internalCallables":[],"gaps":["SOURCE_EVIDENCE_UNAVAILABLE"]});
    };
    let (public, internal): (Vec<_>, Vec<_>) = evidence.entrypoints.iter().partition(|e| {
        !e.kind.starts_with("SOURCE_")
            || e.trigger
                .get("frameworkDeclarations")
                .and_then(Value::as_array)
                .is_some_and(|v| !v.is_empty())
    });
    json!({"publicBoundaries":public,"internalCallables":internal,
        "gaps":["PUBLIC_BOUNDARY_INVENTORY_IS_BOUNDED_BY_SELECTED_MODULES", "DYNAMIC_REGISTRATION_AND_RUNTIME_ACTIVATION_UNVERIFIED"],
        "sourceBoundaries":evidence.boundaries})
}

#[derive(Debug, Subcommand)]
pub enum Command {
    List {
        #[command(flatten)]
        page: ListArgs,
        #[arg(long)]
        service: String,
    },
    Show {
        #[arg(long)]
        root: PathBuf,
        #[arg(long)]
        service: String,
        #[arg(long)]
        id: String,
    },
    Prepare {
        #[arg(long)]
        root: PathBuf,
        #[arg(long)]
        service: String,
        #[arg(long)]
        id: String,
    },
}
pub fn run(command: Command) -> Result<Value, ClewError> {
    let (root, service) = match &command {
        Command::List { page, service } => (&page.root, service),
        Command::Show { root, service, .. } | Command::Prepare { root, service, .. } => {
            (root, service)
        }
    };
    let repo = Repository::open(root)?;
    if !repo.services()?.contains_key(service) {
        return Err(invalid("unknown section service"));
    }
    let baseline = bindings::baseline(&repo)?;
    let subject = format!("service:{service}");
    let rows = records(
        service,
        baseline
            .as_ref()
            .and_then(|(_, b)| b.narratives.get(&subject)),
    );
    match command {
        Command::List { page, .. } => super::cli::page(
            &super::digest(&rows)?,
            rows,
            page.cursor.as_deref(),
            page.limit as usize,
            json!({"inputDigest":repo.input_digest()?}),
        ),
        Command::Show { id, .. } => rows
            .into_iter()
            .find(|r| r["id"] == id)
            .ok_or_else(|| invalid("unknown predefined section")),
        Command::Prepare { id, .. } => {
            let row = rows
                .iter()
                .find(|r| r["id"] == id)
                .ok_or_else(|| invalid("unknown predefined section"))?;
            work::prepare(
                &repo,
                subject,
                serde_json::from_value(row["workRequest"].clone()).map_err(super::io_error)?,
            )
        }
    }
}
