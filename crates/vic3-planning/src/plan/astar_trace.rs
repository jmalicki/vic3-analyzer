//! Temporary A* expand tracing (env-gated).
//!
//! Set `VIC3_PLAN_TRACE=1` (or any non-empty value except `0`/`false`) to log
//! each search expansion. Optional `VIC3_PLAN_TRACE_EVERY=N` (default 100).
//!
//! // TODO: remove this module once GDP search blow-up is diagnosed.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;
use std::time::Instant;

static ENABLED: OnceLock<bool> = OnceLock::new();
static EVERY: OnceLock<u64> = OnceLock::new();
static EXPANDS: AtomicU64 = AtomicU64::new(0);
static STARTED: OnceLock<Instant> = OnceLock::new();

/// Whether expand tracing is on (`VIC3_PLAN_TRACE`).
pub fn enabled() -> bool {
    *ENABLED.get_or_init(|| match std::env::var("VIC3_PLAN_TRACE") {
        Ok(v) => {
            let t = v.trim();
            !(t.is_empty() || t == "0" || t.eq_ignore_ascii_case("false"))
        }
        Err(_) => false,
    })
}

fn every() -> u64 {
    *EVERY.get_or_init(|| {
        std::env::var("VIC3_PLAN_TRACE_EVERY")
            .ok()
            .and_then(|s| s.parse().ok())
            .filter(|&n| n > 0)
            .unwrap_or(100)
    })
}

/// Reset counters (call once at the start of a plan / spot-check).
pub fn reset() {
    if !enabled() {
        return;
    }
    EXPANDS.store(0, Ordering::Relaxed);
    let _ = STARTED.set(Instant::now());
    // OnceLock can only set once; if already set, leave the start clock.
    eprintln!("[astar] trace on (every {}); reset expands", every());
}

/// Log a search expansion (successors / beam chunk). Returns the expand index.
pub fn on_expand(kind: &str, detail: impl FnOnce() -> String) {
    if !enabled() {
        return;
    }
    let n = EXPANDS.fetch_add(1, Ordering::Relaxed) + 1;
    let every = every();
    if n != 1 && !n.is_multiple_of(every) {
        return;
    }
    let secs = STARTED.get_or_init(Instant::now).elapsed().as_secs_f64();
    eprintln!("[astar] #{n} +{secs:.2}s {kind} {}", detail());
}

/// Log when a goal node is closed.
pub fn on_goal(kind: &str, detail: impl FnOnce() -> String) {
    if !enabled() {
        return;
    }
    let n = EXPANDS.load(Ordering::Relaxed);
    let secs = STARTED.get_or_init(Instant::now).elapsed().as_secs_f64();
    eprintln!(
        "[astar] GOAL after {n} expands +{secs:.2}s {kind} {}",
        detail()
    );
}

/// Current expand count (for end-of-search summary).
pub fn expands() -> u64 {
    EXPANDS.load(Ordering::Relaxed)
}
