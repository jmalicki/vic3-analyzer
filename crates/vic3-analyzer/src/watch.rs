//! Debounced filesystem watch over configured save roots.
//!
//! Emits WebView event [`SAVES_CHANGED_EVENT`] so the GUI list refreshes. Does
//! **not** auto-run A* / `plan(...)`. MCP catalog notifications are separate
//! (`docs/mcp.md`).

use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::Duration;

use notify::{RecommendedWatcher, RecursiveMode};
use notify_debouncer_mini::{new_debouncer, DebounceEventResult, DebouncedEventKind};
use tauri::{AppHandle, Emitter};

/// Event name emitted to the WebView when the save catalog should refresh.
pub const SAVES_CHANGED_EVENT: &str = "saves-changed";

/// Watch `save_dirs` and emit [`SAVES_CHANGED_EVENT`] after create/rename bursts.
///
/// # Arguments
///
/// * `app` — handle used to emit events to the WebView.
/// * `roots` — absolute save-root directories; non-directories are skipped.
///
/// Returns [`None`] when there are no watchable dirs or the debouncer cannot start.
pub fn spawn_save_watcher(app: AppHandle, roots: Vec<PathBuf>) -> Option<WatchHandle> {
    let dirs: Vec<PathBuf> = roots.into_iter().filter(|p| p.is_dir()).collect();
    if dirs.is_empty() {
        return None;
    }

    let (tx, rx) = mpsc::channel::<DebounceEventResult>();
    let mut debouncer = new_debouncer(Duration::from_millis(400), move |res| {
        let _ = tx.send(res);
    })
    .ok()?;

    for dir in &dirs {
        if let Err(err) = debouncer.watcher().watch(dir, RecursiveMode::NonRecursive) {
            eprintln!("vic3-analyzer: could not watch {}: {err}", dir.display());
        }
    }

    let app_for_thread = app.clone();
    std::thread::Builder::new()
        .name("vic3-save-watch".into())
        .spawn(move || {
            while let Ok(result) = rx.recv() {
                match result {
                    Ok(events) => {
                        if events.iter().any(|e| is_interesting(&e.kind, &e.path)) {
                            let _ = app_for_thread.emit(SAVES_CHANGED_EVENT, ());
                        }
                    }
                    Err(err) => {
                        eprintln!("vic3-analyzer: watch error: {err}");
                    }
                }
            }
        })
        .ok()?;

    Some(WatchHandle {
        _debouncer: debouncer,
    })
}

fn is_interesting(kind: &DebouncedEventKind, path: &Path) -> bool {
    let looks_like_save = path
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("v3"));
    match kind {
        DebouncedEventKind::Any | DebouncedEventKind::AnyContinuous => looks_like_save,
        _ => looks_like_save,
    }
}

/// Keeps the debouncer alive for the app lifetime.
pub struct WatchHandle {
    _debouncer: notify_debouncer_mini::Debouncer<RecommendedWatcher>,
}

/// Restart watching after Settings changes `save_dirs`.
///
/// # Arguments
///
/// * `current` — previous handle slot (dropped / replaced).
/// * `roots` — new save-root list from config.
pub fn restart_watcher(app: &AppHandle, current: &mut Option<WatchHandle>, roots: Vec<PathBuf>) {
    *current = spawn_save_watcher(app.clone(), roots);
}
