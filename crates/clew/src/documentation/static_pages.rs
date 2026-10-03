//! Offline native-source pages over explicit immutable documentation snapshots.
mod authored;
mod linked;
pub mod model;
mod project;
mod publish;

use super::{
    check::Check,
    invalid,
    store::{self, Repository},
};
use crate::error::ClewError;
use clap::Subcommand;
use serde_json::{Value, json};
use std::path::PathBuf;

pub use project::project;

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Produce linked HTML and inert MDX 3 from native retained source declarations.
    Render {
        #[arg(long)]
        root: PathBuf,
        /// Exact immutable handle returned by docs check. No capture or latest fallback.
        #[arg(long)]
        snapshot: String,
        /// JSON array of exact declaration selectors and optional captured note IDs.
        #[arg(long)]
        input: PathBuf,
        /// New or empty directory; generated files are never overwritten.
        #[arg(long)]
        output: PathBuf,
    },
}

pub fn run(command: Command) -> Result<Value, ClewError> {
    match command {
        Command::Render {
            root,
            snapshot,
            input,
            output,
        } => {
            let repo = Repository::open(&root)?;
            let checked = Check::load_snapshot(&repo, &snapshot)?;
            let selections: Vec<model::Selection> = store::read(&input, 1024 * 1024)?;
            if selections.is_empty() || selections.len() > 128 {
                return Err(invalid(
                    "native pages require 1 to 128 exact declaration selections",
                ));
            }
            let mut projection = project::project_unresolved(&checked, &selections)?;
            authored::attach(&repo, &checked, &mut projection)?;
            let manifest = publish::write(&output, &snapshot, &projection)?;
            Ok(
                json!({"schema":"codeclew-native-pages-render/1.0", "status":"RENDERED",
                "snapshot":snapshot, "inputDigest":projection.input_digest,
                "contextDigest":projection.context_digest, "selectionDigest":projection.selection_digest,
                "projectionDigest":super::digest(&projection)?, "output":output,
                "files":manifest["files"], "manifest":"manifest.json"}),
            )
        }
    }
}
