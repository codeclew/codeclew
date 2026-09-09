//! Durable service documentation, independent from mutation missions and sessions.
pub mod analysis;
pub mod bindings;
pub mod check;
pub mod cli;
pub mod contracts;
pub(crate) mod kotlin;
pub mod model;
pub mod render;
pub mod store;

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
