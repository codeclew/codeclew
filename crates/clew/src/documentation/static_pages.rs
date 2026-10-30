//! Offline native-source pages over explicit immutable documentation snapshots.
mod authored;
mod contract_comparison;
mod data_model;
mod data_state;
mod docusaurus;
mod full;
pub(super) mod inline_contract;
mod linked;
mod markdown;
pub mod model;
mod project;
mod prose_language;
mod publish;
mod question_reading;
mod selected_language;
mod source;

#[path = "static_pages/frozen_export.rs"]
mod frozen_export;

use super::{
    check::Check,
    invalid,
    store::{self, Repository},
};
use crate::error::{ClewError, ErrorCode};
use clap::Subcommand;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

pub use project::project;

#[cfg(test)]
#[test]
#[ignore = "manual full frozen publication replay; requires CODECLEW_FULL_PAGES_* inputs"]
fn full_pages_saved_publication_replay() {
    let path = |name| PathBuf::from(std::env::var(name).expect("explicit replay path required"));
    let result = run(Command::Render {
        source_export: super::source_export::Args {
            source_mode: if std::env::var("CODECLEW_FULL_PAGES_SOURCE_MODE").as_deref()
                == Ok("remote")
            {
                super::source_export::SourceMode::Remote
            } else {
                super::source_export::SourceMode::Embedded
            },
            source_map: std::env::var_os("CODECLEW_FULL_PAGES_SOURCE_MAP").map(PathBuf::from),
            export_subjects: std::env::var("CODECLEW_FULL_PAGES_SUBJECT")
                .ok()
                .into_iter()
                .collect(),
        },
        language: None,
        format: PageFormat::Standalone,
        root: path("CODECLEW_FULL_PAGES_ROOT"),
        snapshot: std::env::var("CODECLEW_FULL_PAGES_SNAPSHOT").unwrap(),
        input: None,
        publication: Some(std::env::var("CODECLEW_FULL_PAGES_PUBLICATION").unwrap()),
        reading_plan: std::env::var_os("CODECLEW_FULL_PAGES_READING_PLAN").map(PathBuf::from),
        contract_comparison: std::env::var_os("CODECLEW_FULL_PAGES_CONTRACT_COMPARISON")
            .map(PathBuf::from),
        data_model: std::env::var_os("CODECLEW_FULL_PAGES_DATA_MODEL").map(PathBuf::from),
        output: path("CODECLEW_FULL_PAGES_OUTPUT"),
    })
    .unwrap();
    assert_eq!(
        result["status"],
        std::env::var("CODECLEW_FULL_PAGES_EXPECTED_STATUS").unwrap_or_else(|_| "RENDERED".into())
    );
    println!("{}", serde_json::to_string(&result).unwrap());
}

#[cfg(test)]
#[test]
#[ignore = "manual replay of a caller-provided retained snapshot; requires CODECLEW_PAGES_REPLAY_* paths"]
fn source_pages_saved_snapshot_replay() {
    let path = |name| PathBuf::from(std::env::var(name).expect("explicit replay path is required"));
    let rendered = run(Command::Render {
        source_export: Default::default(),
        language: None,
        format: PageFormat::Standalone,
        root: path("CODECLEW_PAGES_REPLAY_ROOT"),
        snapshot: std::env::var("CODECLEW_PAGES_REPLAY_SNAPSHOT")
            .expect("explicit snapshot is required"),
        input: Some(path("CODECLEW_PAGES_REPLAY_INPUT")),
        publication: None,
        reading_plan: None,
        contract_comparison: None,
        data_model: None,
        output: path("CODECLEW_PAGES_REPLAY_OUTPUT"),
    })
    .expect("saved SOURCE pages replay must render");
    assert_eq!(rendered["status"], "RENDERED");
    println!("{}", serde_json::to_string(&rendered).unwrap());
}

fn write_if_policy_current(
    repo: &Repository,
    policy_digest: &str,
    output: &Path,
    snapshot: &str,
    projection: &model::BundleProjection,
    language: Option<&str>,
    source_export: Option<(&super::source_export::Policy, &Check)>,
) -> Result<Value, ClewError> {
    let _lock = repo.lock()?;
    if super::endpoint_publication::load(repo)?.digest()? != policy_digest {
        return Err(ClewError::new(
            ErrorCode::WwConflict,
            "endpoint publication policy changed during native render; retry with a new output directory",
        ));
    }
    match source_export {
        Some((policy, checked)) => {
            publish::write_source_policy(output, snapshot, projection, language, policy, checked)
        }
        None => publish::write_language(output, snapshot, projection, language),
    }
}

/// Callable-only projection used by opt-in immutable model Work. Unlike pages,
/// it makes no endpoint/worker/handoff association and selects no live state.
pub(super) fn source_data_graph(
    checked: &Check,
    service: &str,
    declaration: &str,
) -> Result<model::SourceCallGraph, ClewError> {
    source_data_graph_with_depth(checked, service, declaration, 2)
}

pub(super) fn export_publication(
    repo: &Repository,
    checked: &Check,
    snapshot: &str,
    publication: &str,
    language: Option<&str>,
    source_policy: &super::source_export::Policy,
    output: &Path,
) -> Result<Value, ClewError> {
    full::render(
        repo,
        checked,
        snapshot,
        publication,
        full::ReadingInputs {
            reading_plan: None,
            contract_comparison: None,
            data_model: None,
            format: PageFormat::Standalone,
            language,
            source_policy,
        },
        output,
    )
}

pub(super) struct HistoricalInputs<'a> {
    pub language: Option<&'a str>,
    pub data_model: Option<&'a Path>,
    pub reading_plan: Option<&'a Path>,
}

pub(super) fn export_historical_publication(
    repo: &Repository,
    checked: &Check,
    snapshot: &str,
    publication: &str,
    inputs: HistoricalInputs<'_>,
    source_policy: &super::source_export::Policy,
    output: &Path,
) -> Result<Value, ClewError> {
    full::render(
        repo,
        checked,
        snapshot,
        publication,
        full::ReadingInputs {
            reading_plan: inputs.reading_plan,
            contract_comparison: None,
            data_model: inputs.data_model,
            format: PageFormat::Historical,
            language: inputs.language,
            source_policy,
        },
        output,
    )
}

pub(super) fn source_data_graph_with_depth(
    checked: &Check,
    service: &str,
    declaration: &str,
    max_depth: usize,
) -> Result<model::SourceCallGraph, ClewError> {
    let e = checked
        .services
        .get(service)
        .ok_or_else(|| invalid("source-data service is unavailable"))?;
    let d = e
        .observations
        .get(declaration)
        .ok_or_else(|| invalid("source-data root declaration is unavailable"))?;
    if !data_state::supported_language(d) {
        return Err(invalid(
            "sourceDataContext requires an exact retained Java or Kotlin compiler callable",
        ));
    }
    let root_bytes: usize = d
        .source_ids
        .iter()
        .filter_map(|id| e.sources.get(id))
        .map(|s| s.text.len())
        .sum();
    if root_bytes > 1024 * 1024 {
        return Err(invalid(
            "sourceDataContext root exceeds 1 MiB before parsing; narrow the selection",
        ));
    }
    let mut graph = linked::build_roots_with_depth(
        checked,
        &[(service.into(), declaration.into())],
        max_depth,
    )?;
    for node in graph.nodes.values_mut() {
        node.examined_source_digest = linked::node_digest(node)?;
    }
    let selected = graph.nodes.keys().cloned().collect();
    data_state::attach_graph(checked, &mut graph, &selected)?;
    Ok(graph)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum PageFormat {
    Standalone,
    Docusaurus,
    /// Original operation/service navigation with optional field tables.
    Historical,
}

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
        #[arg(
            long,
            required_unless_present = "publication",
            conflicts_with = "publication"
        )]
        input: Option<PathBuf>,
        /// Full saved service/process publication, presented against the selected snapshot.
        #[arg(long, conflicts_with = "input")]
        publication: Option<String>,
        /// Optional source-bound editorial question-reading profile.
        #[arg(long, requires = "publication")]
        reading_plan: Option<PathBuf>,
        /// Retained external HTTP comparison bound to this exact snapshot.
        #[arg(long, requires = "publication")]
        contract_comparison: Option<PathBuf>,
        /// Optional evidence-bound domain model and explicit process Data Flow requests.
        #[arg(long, requires = "publication")]
        data_model: Option<PathBuf>,
        /// Standalone HTML by default; a portable native MDX site for full publications.
        #[arg(long, value_enum, default_value_t = PageFormat::Standalone)]
        format: PageFormat,
        /// Reader language; authored inputs retain their own language and are not translated.
        #[arg(long, value_parser = ["en", "ru"])]
        language: Option<String>,
        #[command(flatten)]
        source_export: super::source_export::Args,
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
            publication,
            reading_plan,
            contract_comparison,
            data_model,
            format,
            language,
            source_export,
            output,
        } => {
            let source_policy = source_export.policy()?;
            super::language::validate(language.as_deref())?;
            let repo = Repository::open(&root)?;
            let checked = Check::load_snapshot(&repo, &snapshot)?;
            if let Some(publication) = publication {
                return full::render(
                    &repo,
                    &checked,
                    &snapshot,
                    &publication,
                    full::ReadingInputs {
                        reading_plan: reading_plan.as_deref(),
                        contract_comparison: contract_comparison.as_deref(),
                        data_model: data_model.as_deref(),
                        format,
                        language: language.as_deref(),
                        source_policy: &source_policy,
                    },
                    &output,
                );
            }
            if !source_policy.subjects.is_empty() {
                return Err(invalid(
                    "export-subject requires --publication; declaration selections do not export authored publications",
                ));
            }
            if format != PageFormat::Standalone {
                return Err(invalid(
                    "the Docusaurus format requires a full saved publication",
                ));
            }
            let input =
                input.ok_or_else(|| invalid("native pages require input or publication"))?;
            let selections: Vec<model::Selection> = store::read(&input, 1024 * 1024)?;
            if selections.is_empty() || selections.len() > 128 {
                return Err(invalid(
                    "native pages require 1 to 128 exact declaration selections",
                ));
            }
            let policy = super::endpoint_publication::load(&repo)?;
            let policy_digest = policy.digest()?;
            let mut projection = project::project_with_policy(&checked, &selections, &policy)?;
            authored::attach(&repo, &checked, &mut projection)?;
            let manifest = write_if_policy_current(
                &repo,
                &policy_digest,
                &output,
                &snapshot,
                &projection,
                language.as_deref(),
                Some((&source_policy, &checked)),
            )?;
            Ok(
                json!({"schema":"codeclew-native-pages-render/1.0", "status":manifest["status"].as_str().unwrap_or("RENDERED"),
                "documentationLanguage":manifest["documentationLanguage"],"requestedDocumentationLanguage":manifest["requestedDocumentationLanguage"],
                "translationGaps":manifest["translationGaps"],"translationComplete":manifest["translationComplete"],
                "snapshot":snapshot, "inputDigest":projection.input_digest,
                "contextDigest":projection.context_digest, "selectionDigest":projection.selection_digest,
                "endpointPublicationPolicyDigest":policy_digest,
                "selectedSelections":projection.pages.len(),
                "excludedSelections":selections.len() - projection.pages.len(),
                "projectionDigest":manifest["projectionDigest"], "sourceExport":manifest["sourceExport"], "exportIdentity":manifest["exportIdentity"], "output":output,
                "files":manifest["files"], "manifest":"manifest.json"}),
            )
        }
    }
}

#[cfg(test)]
#[path = "static_pages/publication_tests.rs"]
mod publication_tests;

/// Shared preflight for a separate export from docs render.
pub(super) fn validate_export_output(output: &Path) -> Result<(), ClewError> {
    full::validate_output(output)
}
