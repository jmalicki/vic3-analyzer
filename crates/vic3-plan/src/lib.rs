//! Planner glue. Search uses `rust_advanced_heaps::pathfinding` (phase 9).
//! Shared option/archive types will live here (phase 5+).
//!
//! Phase 9a toys: [`timed`] graphs with cheap [`timed::TimedNode`] intern keys.
//! Phase 9b: [`Vic3Node`] wires that search API to `vic3-sim`.

/// Ensure the heaps crate stays in the graph (pathfinding API used in phase 9a).
pub use rust_advanced_heaps::pathfinding;

pub mod timed;
pub use timed::{TimedEdge, TimedGraph, TimedNode};

mod vic3;
pub use vic3::Vic3Node;

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
