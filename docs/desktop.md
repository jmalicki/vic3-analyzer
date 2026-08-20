# Desktop config and auto-detect (design spec)

**Status:** Companion UI landed — Settings + save stubs + optional watch + `vic3-api` JSON invokes in `vic3-analyzer`. Advanced Query is Wave 4.  
**Applies to:** Tauri GUI + `vic3-analyzer mcp` (same config file).  
**Does not apply to:** GitHub Pages / wasm (browser still uses pickers / drag-drop).

## Build / run

Desktop crate: `crates/vic3-analyzer` (Tauri 2). Default argv opens the GUI; `mcp` runs stdio MCP via `vic3-mcp` (no window).

```text
cargo check -p vic3-analyzer
cargo test -p vic3-analyzer
cargo run -p vic3-analyzer            # companion UI (Dashboard / Saves / Settings)
cargo run -p vic3-analyzer -- mcp     # stdio MCP (rmcp); logs on stderr, no window
```

Linux CI installs WebKitGTK 4.1 and related packages (see `.github/workflows/ci.yml`). Locally, follow [Tauri prerequisites](https://v2.tauri.app/start/prerequisites/).

The WebView ships `ui/` (companion shell). Invokes use filename stubs in, JSON out via `vic3-api` (paths stay in Rust). To load the Vite `web/` workbench instead, point `build.frontendDist` / `build.devUrl` in `tauri.conf.json` at `web/dist` or the Vite server (set Vite `base` to `/` for the desktop target).

Capability allowlist: `capabilities/default.json` — `core:default` plus `allow-companion` (config, catalog, analysis JSON).

## Goals

- Zero-click defaults: find Victoria 3 **game** data and **save games** on first launch.
- Persist overrides in a config file; expose the same knobs in a Settings UI.
- Never ship Paradox defs or tokens; only paths to the user’s install and files.

## Default discovery

On first launch (or when config paths are missing/invalid), resolve in order:

### Game definitions (`game/`)

Typical Steam layouts (same family as today’s UI hints in `web/src/savePicker.ts`):

| OS | Default candidate |
| --- | --- |
| macOS | `~/Library/Application Support/Steam/steamapps/common/Victoria 3/game` |
| Windows | `C:\Program Files (x86)\Steam\steamapps\common\Victoria 3\game` (and other library roots if detectable) |
| Linux | `~/.local/share/Steam/steamapps/common/Victoria 3/game`, `~/.steam/steam/steamapps/common/Victoria 3/game` |

Validation: directory contains expected allowlisted trees (e.g. `common/`).  
On success: load defs via path-based pipeline (CLI-equivalent); cache postcard or equivalent under **app data**.

### Save catalog roots

| OS | Local saves |
| --- | --- |
| macOS | `~/Documents/Paradox Interactive/Victoria 3/save games` |
| Windows | `%USERPROFILE%\Documents\Paradox Interactive\Victoria 3\save games` |
| Linux | `~/.local/share/Paradox Interactive/Victoria 3/save games` |

Optional Steam Cloud cache (if present):

- `…/Steam/userdata/<id>/529340/remote/save games`

Catalog lists **filename stubs** for MCP/SQL — see [`sql.md`](sql.md) / [`mcp.md`](mcp.md).

### Tokens

No auto-download. If binary/ironman saves are used, Settings points at a user-supplied token map path. Empty = plaintext-only.

## Config file

Stored in platform app data (same root as the CLI archive: `$XDG_DATA_HOME/vic3-analyzer/` or `dirs::data_local_dir()/vic3-analyzer`), as `config.toml` by default. `config.json` is also accepted when present.

Suggested keys:

| Key | Meaning |
| --- | --- |
| `game_dir` | Absolute path to `…/Victoria 3/game` |
| `defs_blob` | Optional absolute path to a prebuilt postcard; if set, may skip live install read |
| `save_dirs` | List of absolute save-root directories |
| `tokens_path` | Optional token map |
| `auto_detect` | Bool; if true, refresh defaults when paths missing |

GUI Settings and MCP both read/write this file. Changing Settings should not require editing SQL.

## Settings UI

| Control | Behavior |
| --- | --- |
| Game folder | Path field + show detected path |
| Use live install vs defs blob | Optional defs postcard path (when set, skips live install) |
| Save folders | Multiline path list (add/remove lines) |
| Token map | Optional path field |
| Reset to auto-detect | Clears overrides and re-runs discovery |

If detection fails: Dashboard “Path hints” modal with pasteable candidates (Cmd+Shift+G / etc.), then write config.

## Tauri commands

| Command | Role |
| --- | --- |
| `get_config` / `save_config` / `reset_config` | Settings round-trip |
| `list_saves` / `get_dashboard` / `detection_hints` | Catalog + status |
| `use_save` | Stub → `vic3-api::load_analysis_from_paths` JSON |
| `loaded_prices` / `loaded_alerts` / `loaded_gaps` | Session analysis JSON |
| `api_ping` | Smoke link to `vic3-api` |

## Watch

Desktop watches configured `save_dirs` (debounced). Emits WebView event `saves-changed` so the GUI list refreshes. MCP `vic3://saves` notifications come with the rmcp server. Does **not** auto-run A\* / `plan(...)`.

## Privacy

Allowlist only configured roots + app data. No scanning of the whole home directory beyond known Steam/Paradox patterns during auto-detect.

## Open questions for review

1. Multiple Steam libraries: how aggressive should discovery be on Windows?
2. Whether defs cache invalidates on game update (mtime of `game/` or version file).

## Implementation notes (non-normative)

- Crate: `vic3-catalog` (shared by future catalog SQL provider, Tauri commands, MCP startup).
- Config format: **TOML default**; JSON supported via extension.
- Pages continues to use IndexedDB defs builder; this doc is native-only.
