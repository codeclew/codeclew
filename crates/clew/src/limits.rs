//! Shared bounded-selection limits for documented and analyzed source.
//!
//! These constants cap the number of module/source-set compilations a single
//! request, session, ready set, discovery pass, documentation service or
//! callable aggregate may select. They are selection-count bounds only: they
//! do not cap concurrent build jobs, fact-index buckets, graph participants,
//! model context or byte budgets, which remain separate limits.

/// Maximum number of selected module/source-set compilations admitted by one
/// request, session, ready set, discovery pass, documentation service or
/// callable aggregate.
pub(crate) const MAX_SELECTED_COMPILATIONS: usize = 128;
