//! Offline, declared-profile interpretation of retained Java flow DSL source.
//! Structural ordering is deliberately separate from runtime execution semantics.
mod extract;
mod publish;
#[cfg(test)]
mod tests;

use super::{
    check::Check,
    digest, invalid,
    model::Source,
    store::{self, Repository},
};
use crate::error::ClewError;
use clap::Subcommand;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{collections::BTreeMap, path::PathBuf};

pub const PROFILE_SCHEMA: &str = "codeclew-flow-dsl-profile/1.0";
pub const PROJECTION_SCHEMA: &str = "codeclew-flow-dsl-projection/1.0";

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Interpret original retained source with a declared Java DSL profile; emit static files.
    Render {
        #[arg(long)]
        root: PathBuf,
        #[arg(long)]
        snapshot: String,
        #[arg(long)]
        service: String,
        #[arg(long)]
        profile: PathBuf,
        /// A new directory, or an existing empty directory. Existing files are preserved.
        #[arg(long)]
        output: PathBuf,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MethodBinding {
    pub file: String,
    /// Exact qualified Java owner, without a class: prefix.
    pub owner: String,
    /// Constructor names use the class name.
    pub method: String,
    /// Prefer this exact retained compiler declaration when supplied and available.
    #[serde(default)]
    pub compiler_symbol: Option<String>,
    #[serde(default)]
    pub scope: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ContextBinding {
    pub file: String,
    pub owner: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CallBinding {
    /// Exact source receiver spelling; qualified target is checked if compiler facts exist.
    pub receiver: String,
    pub method: String,
    #[serde(default)]
    pub compiler_target: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EffectBinding {
    #[serde(flatten)]
    pub call: CallBinding,
    /// mapping, persistence-call, state-change, or opaque-external-call.
    pub kind: String,
    pub label: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum Family {
    BuilderChain,
    OperationQueue,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Profile {
    pub schema: String,
    pub id: String,
    pub title: String,
    /// Authored declaration, not an independently verified engine version.
    pub framework: String,
    pub engine_version: Option<String>,
    pub family: Family,
    pub construction: MethodBinding,
    #[serde(default)]
    pub factory: Option<MethodBinding>,
    #[serde(default)]
    pub registry: Option<MethodBinding>,
    #[serde(default)]
    pub context: Option<ContextBinding>,
    pub operation_type: String,
    pub task_type: String,
    #[serde(default)]
    pub builder: Option<CallBinding>,
    #[serde(default)]
    pub root_variable: Option<String>,
    #[serde(default)]
    pub queue_type: Option<String>,
    #[serde(default)]
    pub initial_argument: Option<usize>,
    #[serde(default)]
    pub registry_receiver: Option<String>,
    /// Explicit calls which select next operations; return expressions are also inspected.
    #[serde(default)]
    pub selection_calls: Vec<CallBinding>,
    #[serde(default)]
    pub decision_methods: Vec<String>,
    #[serde(default)]
    pub effects: Vec<EffectBinding>,
    /// Reader labels are authored profile text; exact source identifiers remain visible.
    #[serde(default)]
    pub labels: BTreeMap<String, String>,
}
impl Profile {
    fn validate(&self) -> Result<(), ClewError> {
        if self.schema != PROFILE_SCHEMA
            || !store::valid_id(&self.id)
            || self.title.trim().is_empty()
            || self.framework.trim().is_empty()
            || self.operation_type.trim().is_empty()
            || self.task_type.trim().is_empty()
        {
            return Err(invalid("flow DSL profile identity or version is invalid"));
        }
        for b in [&self.construction]
            .into_iter()
            .chain(self.factory.iter())
            .chain(self.registry.iter())
        {
            store::relative(&b.file)?;
            if b.owner.is_empty() || b.method.is_empty() {
                return Err(invalid("flow DSL method binding is incomplete"));
            }
        }
        if let Some(c) = &self.context {
            store::relative(&c.file)?;
        }
        match self.family {
            Family::BuilderChain if self.builder.is_none() || self.root_variable.is_none() => {
                return Err(invalid(
                    "builder profile requires API and root variable bindings",
                ));
            }
            Family::OperationQueue
                if self.queue_type.is_none()
                    || self.registry.is_none()
                    || self.registry_receiver.is_none() =>
            {
                return Err(invalid(
                    "queue profile requires constructor and registry bindings",
                ));
            }
            _ => (),
        }
        if self.effects.iter().any(|e| {
            !matches!(
                e.kind.as_str(),
                "mapping" | "persistence-call" | "state-change" | "opaque-external-call"
            )
        }) {
            return Err(invalid("unsupported flow DSL effect kind"));
        }
        if self.labels.len() > 1024 || self.effects.len() > 128 || self.selection_calls.len() > 128
        {
            return Err(invalid("flow DSL profile exceeds its bounded scope"));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Reference {
    pub source_id: String,
    pub start_line: u64,
    pub end_line: u64,
    pub binding_authority: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub compiler_observation: Option<String>,
    pub call_target_authority: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub compiler_call_relation: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Item {
    pub id: String,
    pub kind: String,
    pub label: String,
    pub expression: String,
    /// Ordered members of this declaration only. Registry rows have no sequence meaning.
    pub members: Vec<String>,
    pub conditions: Vec<String>,
    pub reference: Reference,
    pub limitations: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Phase {
    pub id: String,
    pub label: String,
    pub item_ids: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Projection {
    pub schema: String,
    pub id: String,
    pub title: String,
    pub service: String,
    pub revision: String,
    pub snapshot: String,
    pub profile_digest: String,
    pub framework: String,
    pub engine_version: Option<String>,
    pub authority: String,
    pub family: Family,
    pub items: Vec<Item>,
    pub phases: Vec<Phase>,
    pub limitations: Vec<String>,
    pub sources: BTreeMap<String, Source>,
}

pub fn run(command: Command) -> Result<Value, ClewError> {
    match command {
        Command::Render {
            root,
            snapshot,
            service,
            profile,
            output,
        } => {
            let repo = Repository::open(&root)?;
            // Preserve the original loader's integrity/admission errors without translation.
            let checked = Check::load_snapshot(&repo, &snapshot)?;
            let evidence = checked
                .services
                .get(&service)
                .ok_or_else(|| invalid("flow DSL service is absent from the original snapshot"))?;
            let profile: Profile = store::read(&profile, 1024 * 1024)?;
            profile.validate()?;
            let projection = extract::project(evidence, &snapshot, &profile)?;
            publish::write(&output, &projection)?;
            Ok(
                json!({"schema":"codeclew-flow-dsl-render/1.0", "snapshot":snapshot,"service":service,"profileDigest":digest(&profile)?,"projection":projection,"output":output,"files":["projection.json","page.mdx","page.html","style.css","manifest.json","sources.html"]}),
            )
        }
    }
}
