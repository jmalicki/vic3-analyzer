//! Save catalog scan and shared desktop/MCP app config.
//!
//! Pure filesystem helpers for allowlisted Paradox / Steam Cloud save roots and
//! a TOML/JSON config file under the platform app data directory. No network,
//! no DataFusion, no Tauri.

mod catalog;
mod config;
mod error;
mod location;
mod paths;
mod stub;

pub use catalog::{scan_roots, SaveCatalog, SaveEntry};
pub use config::{resolve_config_path, AppConfig};
pub use error::CatalogError;
pub use location::SaveLocation;
pub use paths::{
    app_data_dir, default_config_path, default_game_dir_candidates, default_local_save_dir,
    detect_game_dir, detect_save_roots, detect_steam_cloud_save_dirs, infer_location,
    is_valid_game_dir, roots_from_paths, steam_roots, SaveRoot, APP_NAME, CONFIG_FILE_JSON,
    CONFIG_FILE_TOML, VIC3_STEAM_APP_ID,
};
pub use stub::{classify_kind, normalize_stub, SaveKind, StubError};
