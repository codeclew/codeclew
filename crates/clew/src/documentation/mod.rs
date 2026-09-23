//! Durable service documentation, independent from mutation missions and sessions.
pub mod access;
mod agent_adapter;
pub mod agent_jobs;
pub mod analysis;
pub mod bindings;
pub mod cache;
mod capture_recovery;
pub mod check;
pub mod cli;
pub mod composition;
pub mod contracts;
pub mod dataflow;
pub mod entities;
pub mod evidence_package;
pub mod fact_index;
pub mod history;
mod kotlin;
mod language;
pub mod model;
pub mod notes;
pub mod object_layout;
pub mod plantuml;
pub mod process_candidates;
mod process_context;
pub mod process_flow;
pub mod process_states;
pub mod processes;
pub mod progress;
pub mod proposals;
mod reader;
pub mod render;
pub mod review;
mod section_author;
pub mod sections;
pub mod snapshot_pins;
mod source_inputs;
pub mod source_steps;
pub(crate) mod sqlite_objects;
pub mod status;
pub mod store;
mod syntax;
pub mod updates;
pub mod visuals;
pub mod work;

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

pub mod modules;

mod source_annotations;
