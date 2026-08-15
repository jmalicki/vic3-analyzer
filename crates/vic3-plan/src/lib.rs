//! Planner glue. Search uses `rust_advanced_heaps::pathfinding` (phase 9).
//! Shared option/archive types will live here (phase 5+).
//!
//! Phase 9a toys: [`timed`] graphs with cheap [`timed::TimedNode`] intern keys.

/// Ensure the heaps crate stays in the graph (pathfinding API used in phase 9a).
pub use rust_advanced_heaps::pathfinding;

pub mod timed;
pub use timed::{TimedEdge, TimedGraph, TimedNode};

/// Crate version from Cargo.
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

#[cfg(test)]
mod tests {
    #[test]
    fn version_is_semver() {
        assert!(!super::version().is_empty());
    }
}
