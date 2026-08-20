//! Save catalog scan and shared desktop/MCP app config.
//!
//! Pure filesystem helpers for allowlisted Paradox / Steam Cloud save roots and
//! a TOML/JSON config file under the platform app data directory. No network,
//! no DataFusion, no Tauri.
//!
//! # Role in the stack
//!
//! ```text
//! AppConfig + path auto-detect
//!        │
//!        ▼
//! scan_roots → SaveCatalog (filename stubs, location, mtime)
//!        │
//!        ▼
//! vic3-sql::SqlEngine::use_save  →  active.* / TVFs
//!        │
//!        ├── vic3-analyzer (Tauri companion UI)
//!        └── vic3-mcp      (stdio MCP, same config file)
//! ```
//!
//! Agents and the GUI address saves by **filename stub** (`autosave`, not a
//! path). Absolute paths stay in Rust ([`SaveEntry::path`]) and never cross the
//! WebView / MCP tool boundary as arguments.
//!
//! # Filename stubs
//!
//! [`normalize_stub`] accepts `autosave` or `autosave.v3` and rejects path-like
//! input. [`SaveCatalog::resolve`] returns [`CatalogError::Ambiguous`] when the
//! same stub exists under both `local` and `steam_cloud` roots — callers pass
//! [`SaveLocation`] (and optionally `mtime`) to disambiguate.
//!
//! # Config
//!
//! [`AppConfig`] is the shared Settings / MCP knobs file (`config.toml` by
//! default under [`app_data_dir`]). See `docs/desktop.md`.
//!
//! # See also
//!
//! - `docs/desktop.md` — auto-detect, Settings, Tauri commands
//! - `docs/sql.md` — `saves` table / stub contract
//! - `docs/mcp.md` — MCP `use_save` / `refresh_catalog`

mod catalog;
mod config;
mod error;
mod location;
mod paths;
mod stub;

pub use catalog::{scan_roots, SaveCatalog, SaveEntry};
pub use config::{resolve_config_path, AppConfig, DesktopConfig};
pub use error::CatalogError;
pub use location::SaveLocation;
pub use paths::{
    app_data_dir, default_config_path, default_game_dir_candidates, default_local_save_dir,
    detect_game_dir, detect_save_roots, detect_steam_cloud_save_dirs, infer_location,
    is_valid_game_dir, roots_from_paths, steam_roots, SaveRoot, APP_NAME, CONFIG_FILE_JSON,
    CONFIG_FILE_TOML, VIC3_STEAM_APP_ID,
};
pub use stub::{classify_kind, normalize_stub, SaveKind, StubError};
