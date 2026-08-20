# Desktop config and auto-detect

**Status:** Implemented — companion UI (Dashboard / Saves / Query / Settings) + shared fat binary (`gui` / `mcp` argv); Settings, catalog, SQL, and MCP share config/defs resolution.  
**Applies to:** Tauri GUI + `vic3-analyzer mcp` (same `config.toml`).  
**Does not apply to:** GitHub Pages / wasm (browser pickers / drag-drop).

## Build / run

Desktop crate: `crates/vic3-analyzer` (Tauri 2). **One fat binary:** default argv opens the GUI; `mcp` runs stdio MCP via `vic3-mcp` (early argv branch — **no window**). WebView native libraries may still load at process start because the binary links Tauri; that does not open a window (acceptable for v1; no second MCP artifact).

```text
cargo check -p vic3-analyzer
cargo test -p vic3-analyzer
cargo run -p vic3-analyzer            # companion UI (Dashboard / Saves / Query / Settings)
cargo run -p vic3-analyzer -- mcp     # stdio MCP (rmcp); logs on stderr, no window
./scripts/mcp-smoke.sh                # headless ready check (CI; no display required)
```

Linux CI installs WebKitGTK 4.1 (see `.github/workflows/ci.yml`). Locally: [Tauri prerequisites](https://v2.tauri.app/start/prerequisites/).

### Fat binary / WebView caveat

v1 is **one** artifact linking Tauri + MCP. `main` branches on argv **before** `tauri::Builder::run`, so MCP never creates a window. WebView/runtime libraries may still **map at process start** (startup cost / Windows WebView2 requirement). A second headless binary is deferred.

Capability allowlist: `capabilities/default.json` — `core:default` plus `allow-companion` (config, catalog, analysis JSON, Advanced Query).

The WebView ships `ui/`. Invokes use **filename stubs** in, JSON out (`vic3-api` / `vic3-sql`); absolute save paths stay in Rust.

## How pieces connect

```text
vic3-catalog          AppConfig + scan_roots (stubs, location, mtime)
        │
        ▼
vic3-sql              SqlEngine::use_save → active.* / TVFs
        │
        ├── vic3-analyzer   Tauri companion (CompanionSession)
        └── vic3-mcp        stdio tools (McpRuntime; separate process)
```

GUI and MCP share the **config file** and crates; they do **not** share RAM session in v1.

## Filename stubs

Basename only (`autosave` or `autosave.v3`). No paths. Ambiguous local vs Steam Cloud → error with candidates; disambiguate with `location` / `mtime`. See [`sql.md`](sql.md).

## Default discovery

On first launch (or when config paths are missing/invalid), resolve in order:

### Game definitions (`game/`)

| OS | Default candidate |
| --- | --- |
| macOS | `~/Library/Application Support/Steam/steamapps/common/Victoria 3/game` |
| Windows | `C:\Program Files (x86)\Steam\steamapps\common\Victoria 3\game` (and other library roots if detectable) |
| Linux | `~/.local/share/Steam/steamapps/common/Victoria 3/game`, `~/.steam/steam/steamapps/common/Victoria 3/game` |

Validation: directory contains `common/`. On success: defs via path pipeline; postcard cache under **app data**.

### Save catalog roots

| OS | Local saves |
| --- | --- |
| macOS | `~/Documents/Paradox Interactive/Victoria 3/save games` |
| Windows | `%USERPROFILE%\Documents\Paradox Interactive/Victoria 3\save games` |
| Linux | `~/.local/share/Paradox Interactive/Victoria 3/save games` |

Optional Steam Cloud: `…/Steam/userdata/<id>/529340/remote/save games`.

### Tokens

No auto-download. Empty `tokens_path` = plaintext-only.

## Config file

Platform app data (`$XDG_DATA_HOME/vic3-analyzer/` or `dirs::data_local_dir()/vic3-analyzer`): `config.toml` default; `config.json` accepted when present.

| Key | Meaning |
| --- | --- |
| `game_dir` | Absolute path to `…/Victoria 3/game` |
| `defs_blob` | Optional prebuilt postcard; skips live install read |
| `save_dirs` | Absolute save-root directories |
| `tokens_path` | Optional token map |
| `auto_detect` | Fill missing/invalid paths on load |

## Settings UI

| Control | Behavior |
| --- | --- |
| Game folder | Path + detected path |
| Defs blob | Optional postcard path |
| Save folders | Multiline path list |
| Token map | Optional path |
| Reset to auto-detect | Clears overrides, re-runs discovery |

## Tauri commands

| Command | Role |
| --- | --- |
| `get_config` / `save_config` / `reset_config` | Settings round-trip |
| `list_saves` / `get_dashboard` / `detection_hints` | Catalog + status |
| `use_save` | Stub → `vic3-sql` bind + analysis session |
| `loaded_prices` / `loaded_alerts` / `loaded_gaps` | Session analysis JSON |
| `sql_query` | Read-only SQL → `{ columns, rows, row_count }` |
| `sql_docs` | Body of [`sql.md`](sql.md) + UDF index |
| `api_ping` | Smoke link to `vic3-api` |

## Watch

Debounced watch on `save_dirs` → WebView event `saves-changed`. Does **not** auto-run A\* / `plan(...)`.

## Privacy

Allowlist only configured roots + app data. Auto-detect uses known Steam/Paradox patterns only.
Allowlist only configured roots + app data. No scanning of the whole home directory beyond known Steam/Paradox patterns during auto-detect.

## Open questions for review

1. Multiple Steam libraries: how aggressive should discovery be on Windows?
2. Whether defs cache invalidates on game update (mtime of `game/` or version file).

## Implementation notes (non-normative)

- Crate: `vic3-catalog` (shared by catalog SQL provider, Tauri commands, MCP startup) plus `DesktopConfig` for shared open.
- Defs postcard path: [`vic3_api::ensure_defs_blob`] (GUI + MCP).
- Config format: **TOML default**; JSON supported via extension.
- Pages continues to use IndexedDB defs builder; this doc is native-only.
- Fat binary v1: early argv `mcp` branch; WebView may still load — see [`mcp.md`](mcp.md).
