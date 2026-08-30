//! Temporary A* expand tracing (env-gated).
//!
//! Set `VIC3_PLAN_TRACE=1` (or any non-empty value except `0`/`false`) to log
//! each search expansion. Optional `VIC3_PLAN_TRACE_EVERY=N` (default 100).
//!
//! Field glossary for `[astar]` stderr lines:
//! [`docs/planning-search.md`](../../../../docs/planning-search.md#debug-expand-tracing-vic3_plan_trace).
//! Keep that section in sync when changing line formats here or at call sites.
//!
//! // TODO: remove this module once GDP search blow-up is diagnosed.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

static ENABLED: OnceLock<bool> = OnceLock::new();
static EVERY: OnceLock<u64> = OnceLock::new();
static EXPANDS: AtomicU64 = AtomicU64::new(0);
static STARTED: Mutex<Option<Instant>> = Mutex::new(None);

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

fn elapsed_secs() -> f64 {
    STARTED
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get_or_insert_with(Instant::now)
        .elapsed()
        .as_secs_f64()
}

/// Reset counters (call once at the start of a plan / spot-check).
pub fn reset() {
    if !enabled() {
        return;
    }
    EXPANDS.store(0, Ordering::Relaxed);
    *STARTED.lock().unwrap_or_else(|e| e.into_inner()) = Some(Instant::now());
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
    eprintln!("[astar] #{n} +{:.2}s {kind} {}", elapsed_secs(), detail());
}

/// Log when a goal node is closed.
pub fn on_goal(kind: &str, detail: impl FnOnce() -> String) {
    if !enabled() {
        return;
    }
    let n = EXPANDS.load(Ordering::Relaxed);
    eprintln!(
        "[astar] GOAL after {n} expands +{:.2}s {kind} {}",
        elapsed_secs(),
        detail()
    );
}

/// Current expand count (for end-of-search summary).
pub fn expands() -> u64 {
    EXPANDS.load(Ordering::Relaxed)
}
