# MCP server

**Status:** Implemented (Wave 4b) — stdio MCP via official [rmcp](https://crates.io/crates/rmcp) in `vic3-mcp`, invoked as `vic3-analyzer mcp` from the **same fat binary** as the Tauri GUI.  
**SDK:** Official Rust [rmcp](https://crates.io/crates/rmcp) 3.x.  
**Binary (v1):** `vic3-analyzer` opens the Tauri GUI; `vic3-analyzer mcp` runs stdio MCP **without** creating a window (early argv branch before Tauri `run`). WebView libraries may still load at process start; acceptable for v1 (no second MCP artifact unless headless forces a feature-split later).  
**SQL contract:** [`sql.md`](sql.md) — MCP exposes tables/UDFs; it does not redefine them.  
**Config:** Same file as the GUI ([`desktop.md`](desktop.md)).

## Fat binary

One desktop binary: default argv → Tauri GUI; `vic3-analyzer mcp` → stdio MCP **without** calling Tauri `run` (no window). WebView libraries may still load at process start — acceptable for v1; a second headless artifact is deferred. See [`desktop.md`](desktop.md).

## Process and transport

| Item | Spec |
| --- | --- |
| Invoke | `vic3-analyzer mcp` |
| stdout | Protocol only |
| stderr | Logs (never protocol) |
| Window | Must not open |
| Engine | Shared `vic3-sql` crate (separate process / RAM from GUI) |

On start: load config, refresh save catalog, register DataFusion + tools. No prior GUI launch required.

## Agent flow

1. Discover saves (`query` on `saves`, or resource `vic3://saves`).
2. Bind with `use_save` (stub or selector).
3. Prefer `campaign_brief` for a compact overview, then `query` / `preview_delta` as needed.

Do **not** use `SELECT set_active_save(...)`.

```text
query:  SELECT name, kind, in_game_date, mtime, location FROM saves ORDER BY mtime DESC LIMIT 10
use_save: { "name": "autosave" }
campaign_brief: {}
query:  SELECT * FROM alerts() WHERE severity = 1
        -- player-scoped by default; alerts('all') for the full save
preview_delta: { "building": "building_rye_farm", "extra_levels": 1 }
```

Or: `use_save: { "selector": "latest_autosave" }`.

## Tools

JSON Schema from Rust (rmcp + schemars).

### `query`

| Arg | Type | Required | Notes |
| --- | --- | --- | --- |
| `sql` | string | yes | Single statement; read-only rules in [`sql.md`](sql.md) |
| `format` | string | no | `json` (default) \| `csv` |

**Result:** `{ "columns", "rows", "row_count" }` or CSV. Tool errors for syntax, no active save, DDL, plan timeout.

### `use_save`

| Arg | Type | Required | Notes |
| --- | --- | --- | --- |
| `name` | string | one of name/selector | Stub (`autosave` or `autosave.v3`) |
| `selector` | string | one of name/selector | `latest` \| `latest_autosave` \| `latest_named` |
| `location` | string | no | `local` \| `steam_cloud` |
| `mtime` | string | no | ISO timestamp |

**Result:** `{ "name", "kind", "in_game_date", "country", "loaded": true }`.  
**Errors:** not found; ambiguous stub (candidates with `name`/`kind`/`mtime`/`location`); missing defs/tokens.

### `refresh_catalog`

Rescan allowlisted save dirs → `{ "count": number }`.

### `campaign_brief`

Empty args (`{}`). Requires a bound save (`use_save` first); otherwise a tool error (`no active session binding`).

**Result (compact JSON):**

| Field | Notes |
| --- | --- |
| `session` | `{ name, kind, in_game_date, country }` from the active session |
| `player_tag` | Bound world's played country tag (may be null) |
| `top_goods` | Up to ~10 domestic goods by summed shortage (`owner_tag` = player) |
| `hotspots` | Up to ~15 `{ region_name, good, shortage }` rows |
| `alert_kinds` | Kind → count histogram from **player-default** alerts (same filter as zero-arg `alerts()`, not `alerts('all')`) |

Built from `SqlEngine::active_binding()` (no SQL round-trip).

### `explain`

| Arg | Type | Notes |
| --- | --- | --- |
| `sql` | string | Returns DataFusion explain text |

### `preview_delta`

Session-bound what-if / [`WorldDelta`](json-schema.md) preview (same solver as CLI `mutate` / `what-if`). Requires a prior `use_save`. Warm-starts from the bound solve’s `relative` prices. Does **not** mutate the session.

Provide **either** sugar **or** `delta` (not both; unknown fields rejected).

| Arg | Type | Required | Notes |
| --- | --- | --- | --- |
| `building` | string | sugar | Building type id (e.g. `building_rye_farm`) |
| `extra_levels` | number | sugar | Non-negative extra levels |
| `building_id` | number | no | Instance id (wins over `building`) |
| `state_id` | number | no | Restrict type match to one state |
| `delta` | object | alt | Full WorldDelta (`extra_levels` / `production_methods` / `subsidize`) |

**Result (compact only — not a full PricesResult):**

```json
{
  "status": "converged",
  "residual": 1e-9,
  "applied": { "extra_levels": [{ "building": "building_rye_farm", "extra_levels": 1 }] },
  "goods": [
    {
      "id": "wood",
      "price_before": 40.0,
      "price_after": 35.0,
      "d_price": -5.0,
      "shortage_before": 0.0,
      "shortage_after": 0.0,
      "d_shortage": 0.0
    }
  ],
  "limitations": ["…"]
}
```

`goods` lists rows whose price or shortage changed. Shortage is `max(0, buy − sell)` (same as SQL goods tables).

**Errors:** no active save; sugar and `delta` both set; incomplete sugar; unknown JSON fields.

## Resources

| URI | Content |
| --- | --- |
| `vic3://schema` | Tables / columns / TVFs (same registry as SQL) |
| `vic3://saves` | Catalog snapshot (no absolute paths) |
| `vic3://session` | Active stub, defs status |
| `vic3://docs/flow` | Short list → use_save → campaign_brief → query |
| `vic3://docs/sql` | Body of [`sql.md`](sql.md) |
| `vic3://docs/mcp` | Body of this file |

## Prompts

| Prompt | Purpose |
| --- | --- |
| `investigate_shortages` | use_save → `campaign_brief` → optional domestic shortage SQL |
| `compare_latest_autosave` | Catalog + bind + summary |
| `military_readiness` | Military queries when available |
| `what_is_loaded` | `vic3://session` + counts |
| `plan_research` | `plan('research(tech=…)')` |

Reminders in prompt text: stubs not paths; read-only SQL; `use_save` before fact tables.

## Completions

- Prompt/resource args `name` / `stub` / `save`: catalog stubs
- Optional: SQL table names for `table` arguments

## Security

| Rule | Detail |
| --- | --- |
| Allowlist | Configured game dir, save dirs, app data, optional tokens |
| No path args | Tools accept stubs/selectors only |
| Read-only SQL | Enforced in query layer |
| AGPL | Local process; no network service required |

## Client configuration

```json
{
  "mcpServers": {
    "vic3-analyzer": {
      "command": "/path/to/vic3-analyzer",
      "args": ["mcp"]
    }
  }
}
```

macOS app bundle: `…/Vic3 Analyzer.app/Contents/MacOS/vic3-analyzer` with args `["mcp"]`.

## Consistency with GUI

| Concern | Behavior |
| --- | --- |
| Config file | Shared |
| Catalog / SQL | Same crates; separate process instance |
| Active save | Per process |
| Result shape | `sql_query` invoke ↔ MCP `query` JSON |
| Catalog | Shared code; MCP process has its own instance unless later we add a daemon |
| Active save | Per process (GUI and MCP do not share RAM session in v1) |
| SQL engine | Same `vic3-sql` crate and [`sql.md`](sql.md) contracts |

## Open questions for review

1. Should `query` auto-bind `latest` if no session, or always require `use_save`?
2. Max rows / timeout defaults for `query` and `plan(...)`.
3. Whether GUI and MCP should ever share a long-lived daemon (out of v1).

## Implementation notes

- Crate: `vic3-mcp`, linked into the `vic3-analyzer` binary (`mcp` argv).
- Tools / resources / prompts implemented; catalog watch → `list_changed` remains best-effort follow-up.
- Shared with GUI: [`vic3_catalog::DesktopConfig`] + [`vic3_api::ensure_defs_blob`] (same config file and defs cache path). Active SQL session stays per-process.
- Headless startup smoke: `./scripts/mcp-smoke.sh` (CI) asserts `mcp` reaches ready on stderr without a display / without calling Tauri `run`.
- Protocol smoke client: `./scripts/mcp_smoke.py` (NDJSON stdio) — see [Smoke client](#smoke-client) below.

## Smoke client

In-repo NDJSON client for a full tool round-trip against a running `vic3-analyzer mcp` process ([#46](https://github.com/jmalicki/vic3-analyzer/issues/46)). Distinct from `mcp-smoke.sh` (startup / no-window only).

**Flow:** `initialize` → `notifications/initialized` → `tools/list` → `use_save` → then:

| Condition | Next step |
| --- | --- |
| `campaign_brief` in tools | call `campaign_brief` |
| else | `query` a small domestic shortage SQL (`owner_tag = player_tag()`) |
| `--preview-rye` and `preview_delta` present | sugar `building_rye_farm` / `extra_levels: 1` |

Optional advisor tools are detected from `tools/list`, so the script works on **main** (`query` + `use_save`) and against branches that add `campaign_brief` / `preview_delta` without hard-requiring them. Prints one JSON object on stdout; exits `1` if a tool returns `isError` or the protocol fails. Server logs stay on stderr.

### Invoke

```bash
# Binary: first arg, VIC3_MCP_BIN, VIC3_ANALYZER_BIN, or vic3-analyzer on PATH
export VIC3_MCP_BIN=./target/debug/vic3-analyzer

python3 scripts/mcp_smoke.py --name autosave
python3 scripts/mcp_smoke.py --selector latest_autosave --location local
python3 scripts/mcp_smoke.py --name autosave --preview-rye   # no-op skip if tool missing
```

### Fixture smoke (local)

Use a private `XDG_DATA_HOME` so the process does not touch a live Steam install. Same fixture inputs as `vic3-mcp` tool tests:

```bash
TMP=$(mktemp -d)
export XDG_DATA_HOME="$TMP/xdg"
APP="$XDG_DATA_HOME/vic3-analyzer"
mkdir -p "$APP" "$TMP/saves"
cp crates/vic3-load/tests/fixtures/plaintext.txt "$TMP/saves/autosave.v3"
cargo run -p vic3-defs --bin emit_fixture_blob -- "$APP/defs.postcard"
cat >"$APP/config.toml" <<EOF
auto_detect = false
defs_blob = "$APP/defs.postcard"
save_dirs = ["$TMP/saves"]
EOF

cargo build -p vic3-analyzer
export VIC3_MCP_BIN=./target/debug/vic3-analyzer
python3 scripts/mcp_smoke.py --name autosave
```

Without a configured catalog/defs, `use_save` errors and the script exits `1`.