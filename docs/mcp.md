# MCP server (design spec)

**Status:** Wave 0 design for review. Not implemented yet.  
**SDK (planned):** Official Rust [rmcp](https://crates.io/crates/rmcp) (Model Context Protocol).  
**Binary (v1):** Single fat desktop binary — `vic3-analyzer` opens the Tauri GUI; `vic3-analyzer mcp` runs stdio MCP **without** creating a window (early argv branch before Tauri `run`). WebView libraries may still load at process start; acceptable for v1.  
**SQL contract:** [`sql.md`](sql.md) — MCP does not redefine tables/UDFs; it exposes them.

## Goals

- Let LLM clients answer freeform campaign questions by calling tools (especially SQL).
- Share catalog, config, and analysis session with the Tauri GUI when using the same install/config.
- Keep saves and defs on-device (AGPL privacy story): allowlisted dirs only; no upload.

## Non-goals (v1)

- Sampling, elicitation, or exposing arbitrary filesystem roots.
- Write tools that overwrite live autosaves (no `export_save` tool until explicitly designed).
- A second MCP-only binary (deferred).
- Embedding MCP inside the Pages wasm app.

## Process and transport

| Item | Spec |
| --- | --- |
| Invoke | `vic3-analyzer mcp` (stdio JSON-RPC per MCP) |
| stdout | Protocol only |
| stderr | Logs (never protocol) |
| Window | Must not open |
| Config | Same app config as GUI ([`desktop.md`](desktop.md) when present): game path, saves roots, optional defs blob, tokens |

On start: load config, refresh save catalog (auto-detect defaults), register DataFusion + tools. Do not require a prior GUI launch.

## Canonical agent flow

1. Discover saves (tool `query` or resource `vic3://saves`).
2. Bind session with `use_save` (stub or selector).
3. `query` analysis SQL against `active` tables / TVFs per [`sql.md`](sql.md).

Do **not** document `SELECT set_active_save(...)`.

### Example

```text
query:  SELECT name, kind, in_game_date, mtime, location FROM saves ORDER BY mtime DESC LIMIT 10
use_save: { "name": "autosave" }
query:  SELECT * FROM alerts() WHERE severity = 1
query:  SELECT step, day, action, detail FROM plan('research(tech=nitroglycerin)') ORDER BY step
```

Or skip discovery:

```text
use_save: { "selector": "latest_autosave" }
query: …
```

## Tools

JSON Schema for arguments should be generated from Rust types (rmcp + schemars). Normative shapes below.

### `query`

Run one read-only SQL statement.

| Arg | Type | Required | Notes |
| --- | --- | --- | --- |
| `sql` | string | yes | Single statement; see [`sql.md`](sql.md) read-only rules |
| `format` | string | no | `json` (default) \| `csv` |

**Result:** `{ "columns": [string], "rows": [ ... ], "row_count": number }` or CSV text. Errors as MCP tool errors with a clear message (syntax, no active save, DDL rejected, plan timeout).

### `use_save`

Bind the analysis session.

| Arg | Type | Required | Notes |
| --- | --- | --- | --- |
| `name` | string | one of name/selector | Filename stub (`autosave` or `autosave.v3`) |
| `selector` | string | one of name/selector | `latest` \| `latest_autosave` \| `latest_named` |
| `location` | string | no | `local` \| `steam_cloud` — disambiguate stub |
| `mtime` | string | no | ISO timestamp — further disambiguation |

**Result:** `{ "name", "kind", "in_game_date", "country", "loaded": true }` after load/solve.  
**Errors:** not found; ambiguous stub (include candidates with `name`, `kind`, `mtime`, `location`); missing defs/tokens with actionable message.

Loading may take noticeable time; emit MCP progress notifications when supported.

### `refresh_catalog`

Rescan allowlisted save directories.

**Result:** `{ "count": number }` or summary list.

### `explain` (optional v1)

| Arg | Type | Notes |
| --- | --- | --- |
| `sql` | string | |

Returns DataFusion explain plan text for debugging agent SQL.

## Resources

Support `resources/list` and `resources/read`. Prefer `resources/subscribe` (or list_changed notifications) for `vic3://saves` when the catalog watch fires.

| URI | Content |
| --- | --- |
| `vic3://schema` | Tables, columns, UDFs/TVFs (generated from the same registry as SQL; must match [`sql.md`](sql.md)) |
| `vic3://saves` | Current catalog snapshot (agent-facing columns) |
| `vic3://session` | Active stub, date, country, loaded flags, defs status |
| `vic3://docs/flow` | Short markdown: list → use_save → query |
| `vic3://docs/sql` | Body of [`sql.md`](sql.md) (or rendered excerpt) |
| `vic3://docs/mcp` | Body of this file (or excerpt) |

## Prompts

Suggested MCP prompts (names stable):

| Prompt | Purpose |
| --- | --- |
| `investigate_shortages` | Guide: use_save latest → alerts / shortage_analysis |
| `compare_latest_autosave` | Catalog + bind + summary queries |
| `military_readiness` | Military / formations queries when available |
| `what_is_loaded` | Read `vic3://session` + simple counts |
| `plan_research` | `plan('research(tech=…)')` pattern |

Prompt text should remind the model: stubs not paths; read-only SQL; call `use_save` before fact tables.

## Completions

- `use_save.name`: catalog stubs
- Optional: SQL table/column names from schema registry

## Logging and notifications

- MCP logging level configurable; default info on stderr + MCP log messages if the client supports them
- Notify when catalog changes (new autosave detected)
- Progress on `use_save` / long `plan(...)` queries when the protocol allows

## Security

| Rule | Detail |
| --- | --- |
| Allowlist | Configured game dir, save dirs (Paradox + optional Steam Cloud), app data, optional token path |
| No path args | Tools accept stubs/selectors only |
| Read-only SQL | Enforce in query layer |
| Secrets | Do not echo full token map paths in tool results if avoidable; never upload |
| AGPL | Local process; no network service required |

## Client configuration (illustrative)

Cursor / Claude-style MCP config:

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

On macOS app bundles, `command` may be `…/Vic3 Analyzer.app/Contents/MacOS/vic3-analyzer` with args `["mcp"]`.

## Consistency with GUI

| Concern | Behavior |
| --- | --- |
| Config file | Shared |
| Catalog | Shared code; MCP process has its own instance unless later we add a daemon |
| Active save | Per process (GUI and MCP do not share RAM session in v1) |
| SQL engine | Same `vic3-sql` crate and [`sql.md`](sql.md) contracts |

## Open questions for review

1. Should `query` auto-bind `latest` if no session, or always require `use_save`?
2. Max rows / timeout defaults for `query` and `plan(...)`.
3. Whether GUI and MCP should ever share a long-lived daemon (out of v1).

## Implementation notes (non-normative)

- Crate: part of the desktop binary or `vic3-mcp` module linked into it.
- Wave order: implement after [`sql.md`](sql.md) review and `feat/catalog-sql` / DataFusion land.
