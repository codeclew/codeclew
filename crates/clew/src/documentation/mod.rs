//! Durable service documentation, independent from mutation missions and sessions.
pub mod access;
mod agent_adapter;
pub mod agent_jobs;
pub mod analysis;
mod answer_context;
mod answer_reuse;
mod answer_reuse_projection;
#[cfg(test)]
mod answer_reuse_tests;
pub mod bindings;
pub mod cache;
mod capture_export;
mod capture_recovery;
pub mod check;
pub mod cli;
pub mod composition;
pub mod contracts;
mod csharp;
pub mod dataflow;
mod endpoint_context;
pub mod entities;
pub mod evidence_package;
mod explanation_authorship;
pub mod fact_index;
pub mod flow_dsl;
pub mod history;
mod job_context;
mod kotlin;
mod language;
mod local_cfg;
pub mod maintained_context;
pub mod model;
pub mod model_ids;
#[cfg(test)]
mod model_ids_integration_tests;
pub mod notes;
pub mod object_layout;
mod operation_answer;
mod operation_context;
mod operation_packet;
pub mod plantuml;
pub mod process_candidates;
mod process_context;
pub mod process_flow;
mod process_graph;
pub mod process_states;
pub mod processes;
pub mod progress;
pub mod proposals;
mod reader;
pub mod render;
pub mod review;
pub mod reviewed_answers;
mod section_author;
pub mod sections;
pub mod snapshot_pins;
mod source_data_context;
mod source_inputs;
pub mod source_steps;
pub(crate) mod sqlite_objects;
pub mod static_pages;
pub mod status;
pub mod store;
mod syntax;
mod syntax_file_cache;
#[cfg(test)]
mod syntax_file_cache_tests;
pub mod updates;
pub mod visuals;
pub mod work;
mod work_parts;
mod work_retained_parts;

use crate::error::{ClewError, ErrorCode};
use serde::Serialize;

pub(crate) fn invalid(message: impl Into<String>) -> ClewError {
    ClewError::new(ErrorCode::InvalidInput, message)
}
pub(crate) fn io_error(_: impl std::fmt::Display) -> ClewError {
    ClewError::new(
        ErrorCode::InvalidInput,
        "documentation input/output is unavailable",
    )
}
pub(crate) fn digest(value: &impl Serialize) -> Result<String, ClewError> {
    crate::canonical::hash(value).map_err(io_error)
}
pub(crate) fn bytes(value: &impl Serialize) -> Result<Vec<u8>, ClewError> {
    crate::canonical::bytes(value).map_err(io_error)
}
/// Compiler declarations in the portable JVM identity grammar: javac facts and
/// Roslyn facts projected by `csharp::project_facts`.
pub(crate) fn is_compiler_declaration_schema(schema: &serde_json::Value) -> bool {
    schema == "codeclew-java-compiler-fact/1.0" || schema == csharp::CSHARP_FACT_SCHEMA
}

pub mod modules;

mod source_annotations;
