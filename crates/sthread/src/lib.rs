pub mod agent_context;
pub mod canonical;
pub mod error;
pub mod freshness;
pub mod graph;
pub mod identity;
pub mod index;
pub mod model;
pub mod projection;
pub mod semantic_goal;
pub mod semantic_kernel;
pub mod task_context;
pub mod task_plan;
pub mod thread_projection;
pub mod transaction;
pub mod worker;

pub mod proto {
    include!(concat!(env!("OUT_DIR"), "/semantic_thread.worker.v1.rs"));
}
