pub mod adapter_v2;
pub mod canonical;
pub mod cas;
pub mod cold_start;
pub mod context_v2;
pub mod derived_manifest;
pub mod error;
pub mod freshness;
pub mod generation_service;
pub mod generation_v2;
pub mod identity;
pub mod incremental_v2;
pub mod index;
pub mod kotlin_adapter_v2;
pub mod process_isolation;
pub mod query_v2;
pub mod repository_snapshot;
pub mod runtime;
pub mod session;
pub mod state;
pub mod task_run_v2;
pub mod worker;

pub mod proto {
    include!(concat!(env!("OUT_DIR"), "/semantic_thread.worker.v1.rs"));
}
