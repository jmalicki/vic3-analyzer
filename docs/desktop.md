# Desktop Companion App

The native desktop companion (`crates/vic3-analyzer`) is a [Tauri 2](https://v2.tauri.app/) application providing automatic game and save discovery, a campaign dashboard, and an embedded [DataFusion SQL](sql.md) engine.

The compiled binary serves as both the **Desktop GUI** (default) and the headless **Stdio MCP Server** (`vic3-analyzer mcp`).

---

## Running the Desktop Companion

```bash
# Launch the Tauri GUI companion
cargo run -p vic3-analyzer

# Run the Stdio MCP server (no GUI window opened)
cargo run -p vic3-analyzer -- mcp

# Headless CI ready check
./scripts/mcp-smoke.sh
```

*(For Linux prerequisites such as WebKitGTK 4.1, refer to the [Tauri prerequisites guide](https://v2.tauri.app/start/prerequisites/)).*

---

## Automatic Discovery Paths

On first launch (or when paths are unconfigured), `vic3-analyzer` automatically detects game installs and local save folders:

### 1. Game Definitions (`game/`)

| OS | Default Search Locations |
| --- | --- |
| **macOS** | `~/Library/Application Support/Steam/steamapps/common/Victoria 3/game` |
| **Windows** | `C:\Program Files (x86)\Steam\steamapps\common\Victoria 3\game` (and additional Steam library folders) |
| **Linux** | `~/.local/share/Steam/steamapps/common/Victoria 3/game`, `~/.steam/steam/steamapps/common/Victoria 3/game` |

*Validation:* The folder must contain a `common/` directory. When found, definitions are indexed and cached locally under application data.

### 2. Save Game Directories

| OS | Local Save Folder |
| --- | --- |
| **macOS** | `~/Documents/Paradox Interactive/Victoria 3/save games` |
| **Windows** | `%USERPROFILE%\Documents\Paradox Interactive\Victoria 3\save games` |
| **Linux** | `~/.local/share/Paradox Interactive/Victoria 3/save games` |

*Steam Cloud Saves (Optional):* `.../Steam/userdata/<user-id>/529340/remote/save games`.

---

## Configuration File

Settings are saved in your platform's local application data directory (`$XDG_DATA_HOME/vic3-analyzer/` on Linux/macOS or `%LOCALAPPDATA%\vic3-analyzer` on Windows) as `config.toml` (or `config.json`):

| Key | Type | Description |
| --- | --- | --- |
| `game_dir` | string (path) | Absolute path to the installed `Victoria 3/game` directory |
| `defs_blob` | string (path) | Optional path to a precompiled `defs.postcard` blob |
| `save_dirs` | array of paths | List of directories monitored for `.v3` save files |
| `tokens_path` | string (path) | Optional path to a token mapping file for binary Ironman saves |
| `auto_detect` | boolean | Automatically populate missing paths on startup (default `true`) |

---

## Desktop Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                        vic3-catalog                         │
│           (AppConfig + File Discovery & Watchers)           │
└──────────────────────────────┬──────────────────────────────┘
                               │
                               ▼
┌─────────────────────────────────────────────────────────────┐
│                          vic3-sql                           │
│        (DataFusion SQL Engine + Active Save Session)        │
└──────────────┬──────────────────────────────┬───────────────┘
               │                              │
               ▼                              ▼
┌──────────────────────────────┐┌─────────────────────────────┐
│        vic3-analyzer         ││          vic3-mcp           │
│    (Tauri 2 Companion UI)    ││ (Stdio MCP Agent Server)    │
└──────────────────────────────┘└─────────────────────────────┘
```

- **Shared Configuration:** The GUI and MCP server share the same configuration file and definition caches.
- **Save Watcher:** Monitors configured save directories with debouncing and automatically refreshes the catalog when a new autosave or manual save appears.
- **Privacy Guarantee:** The application only scans allowlisted save and Steam directories. It never transmits saves, telemetry, or system files over the network.

---

## Settings & Commands

The Desktop UI provides settings management and exposes the following internal Tauri commands:
- `get_config` / `save_config` / `reset_config`: Manage application settings.
- `list_saves` / `get_dashboard`: Fetch cataloged saves and summary campaign metrics.
- `use_save`: Bind an active save session for SQL and diagnostics.
- `sql_query`: Execute read-only DataFusion queries against the loaded save.
- `loaded_prices` / `loaded_alerts` / `loaded_gaps`: Retrieve cached analytical projections.
