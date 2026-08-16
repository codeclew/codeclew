//! Language-neutral, snapshot-bound semantic evidence protocol.
//!
//! This crate owns transport, provenance, coverage and decision invariants. It
//! intentionally delegates all language semantics to versioned adapters.

pub mod protocol {
    include!(concat!(env!("OUT_DIR"), "/codeclew.evidence.v1.rs"));
}

mod canonical;
mod contract;
mod policy;
mod registry;
mod validation;

pub use canonical::{
    CanonicalError, canonical_json, canonical_json_bytes, evidence_merkle_root, sha256_digest,
};
pub use contract::{ContractDigests, ContractError, ContractFile, FrozenCoreContract};
pub use policy::{DecisionPolicy, ObligationClosureContract, PolicyError};
pub use registry::{CapabilityQuery, ContractRegistry, RegistryError};
pub use validation::{
    ContentAddressed, EvidenceBundle, Validate, ValidationError, ValidationErrors,
    seal_content_digest, validate_bundle,
};
