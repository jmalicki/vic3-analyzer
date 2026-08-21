# SQL interface (design spec)

**Status:** Wave 4a — Advanced Query tab in the Tauri companion uses this dialect via `vic3-sql`. Wave 3 diagnostics/planning UDFs, `saves` catalog, host `use_save`, and `active.*` / `latest.*` views remain in `vic3-sql`.  
**Engine:** Apache DataFusion (`datafusion` 51).  
**Consumers:** MCP `query` tool (Wave 4b), Tauri Advanced Query tab, optional `vic3-analyzer sql` debug.  
**Related:** [`mcp.md`](mcp.md) (agent transport), [`json-schema.md`](json-schema.md) (JSON field names), [`dsl.md`](dsl.md) / [`planning.md`](planning.md) (goals / A\*), [`desktop.md`](desktop.md) (companion invokes).

This document is the contract implementers must follow. Prefer amending this file over silently changing behavior.

## Goals

- Let humans and LLMs ask ad-hoc questions over a **loaded** campaign with ordinary SQL.
- Expose game objects as **tables** and complex analysis as **scalar / table-valued functions** (TVFs).
- Address saves by **filename stub**, not filesystem paths or opaque UUIDs in the happy path.
- Keep SQL **read-only**. Session binding is a host API (`use_save`), never a mutating `SELECT`.

## Non-goals

- Reimplementing Basin / A\* inside the SQL engine (TVFs call existing Rust).
- DDL/DML (`CREATE`, `INSERT`, `UPDATE`, `DELETE`, `ATTACH` of arbitrary files).
- Path arguments in SQL (`'/Users/.../autosave.v3'`).
- Side-effecting UDFs (`SELECT set_active_save(...)`).
- Shipping Paradox defs or token maps.
- Running `plan(...)` on every autosave (explicit invoke only).
- Browser/Pages embedding of this engine in v1 (native desktop + MCP first).

## Session model

Exactly one **active save** (optional until `use_save`) plus a **catalog** of known local saves.

| Mechanism | Role |
| --- | --- |
| Host `use_save` | Bind session by stub or selector; may load/solve. See [`mcp.md`](mcp.md). |
| Table `saves` | Read-only catalog (stubs + metadata). |
| Views `active.*` | Fact tables for the bound session (no `save_id` column). |
| Views `latest.*` | Convenience: same shape as `active.*`, but pinned to “most recent by `mtime`” at **read** time without mutating session. Document that `mtime` ≠ in-game date. |

Default search path after `use_save`: unqualified `states` means `active.states`.

## Filename stubs

| Rule | Detail |
| --- | --- |
| Form | Basename only. No `/`, `\`, or parent segments. |
| Matching | Accept `autosave` or `autosave.v3`; normalize by stripping one trailing `.v3`. |
| Ambiguity | Same stub in local Documents vs Steam Cloud → error listing `name`, `kind`, `mtime`, `location` (`local` \| `steam_cloud`). Caller disambiguates with those fields — still no UUID in the happy path. |
| Internal | Absolute `path`, fingerprint, cache id exist in Rust only; not agent-facing columns by default. |

Selectors (host API, not SQL): `latest`, `latest_autosave`, `latest_named`.

## Read-only enforcement

The query entrypoint must reject (error, no partial exec):

- Statements other than `SELECT` / `WITH`…`SELECT` / `EXPLAIN` (if `explain` is enabled)
- References to filesystem tables outside the registered providers
- UDFs marked mutating (none in v1)

## Catalog table: `saves`

Agent-facing columns:

| Column | Type | Notes |
| --- | --- | --- |
| `name` | UTF-8 text | Filename stub (primary handle) |
| `kind` | text | `autosave` \| `named` \| `ironman` \| … |
| `mtime` | timestamp | Filesystem mtime |
| `in_game_date` | text or date | From cheap parse or last load; null if unknown |
| `country` | text | Player tag if known |
| `location` | text | `local` \| `steam_cloud` \| … |
| `loaded` | bool | True if this stub is the active session |

### Example

```sql
SELECT name, kind, in_game_date, mtime, location
FROM saves
ORDER BY mtime DESC
LIMIT 10;
```

## Fact tables (`active.*` / `latest.*`)

Rows project from the loaded `World` + `PricesResult` + defs after a solve — not raw Clausewitz. Column names should align with [`json-schema.md`](json-schema.md) / `PricesResult` where practical.

### `states`

Identity and geography for states in scope of the active save.

| Column | Notes |
| --- | --- |
| `state_id` | Integer Paradox id; **primary key** for joins |
| `region_id` | Non-localized region/script key when available (`region` string as stored) |
| `region_name` | Localized display label when available; otherwise a humanized `region_id` (e.g. `STATE_BRANDENBURG` → `Brandenburg`) |
| `owner_tag` | Country tag |
| `market_id` | If modeled |
| `infrastructure` | As exposed in prices result |
| `arable_land` | If present |

**Storage today (Rust):** `World.states` / `PricesResult.states` are a **`Vec` of rows**, each with its own `id: u32`. Sparse Paradox ids. Save IR: `HashMap<u32, Option<State>>`. No state name btree today — SQL may build a **HashMap** for Exact `region_id` / `region_name` equality at bind (not range Exact).

**Ordering:** prefer scan ordered by `state_id` for sort-merge on id joins.

### `goods`

Market-level goods from `PricesResult.goods` / `GameDefs`.

| Column | Notes |
| --- | --- |
| `good` | Script id; **key** — `goods_order` + `index_of` (Exact `=`). Defs also keep a `BTreeMap` by script id → if that map is the scan source, Exact **range** on `good` is allowed |
| `good_name` | Localized (`labels`); Exact `=` via inverse hash if built — **no** range Exact unless a btree of labels exists |
| `base` / `price` / `buy` / `sell` / `shortage` | As modeled; document shortage formula in implementation |

### `goods_by_state`

From `state_goods` (state-attributed orders / MAPI blend).

| Column | Notes |
| --- | --- |
| `state_id` | **key** (part) |
| `good` | **key** (part) |
| `price` | State blended price |
| `buy` / `sell` | Attributed orders |
| `shortage` | As defined for state scope |

### Nested columns vs child tables (normative)

**Match Rust storage.** If the engine already holds a `Vec` / list on the parent row, expose it as a DataFusion **List** (array) column on that table. Do **not** invent a many↔many junction table only for SQL shape.

| Prefer | When |
| --- | --- |
| **Array / list column** on the parent | Field is already `Vec<…>` (or small parallel vecs) on the struct we scan |
| **Child / junction table** | We already store a separate collection keyed for lookup, **or** agents must equijoin/filter/aggregate on exploded elements as first-class rows and UNNEST is too awkward for the common query |

**Arrays stand in for junction tables.** DataFusion can explode lists into rows with `unnest` / `UNNEST` (and `unnest_outer` when empty/NULL parents should keep a row).

Goods IO and PM recipes use **`List<Struct{good, good_name, qty}>`** (script id — never bare `GoodIdx` as the only key; localized label; quantity):

```sql
-- One unnest → rows of structs; double unnest → multi-column (list then struct)
SELECT building_id, unnest(unnest(input_goods))
FROM buildings;
-- → building_id | good | good_name | qty

SELECT pm, unnest(unnest(outputs))
FROM production_methods;
-- → pm | good | good_name | qty
```

Simple `TEXT[]` columns need only one unnest:

```sql
SELECT building_id, unnest(production_methods) AS pm
FROM buildings;
```

**Caveat (DataFusion today):** `FROM parent p, UNNEST(p.col)` lateral style is **not** supported yet — use `unnest` in the **SELECT** list (or a view that does that). Do not teach the Postgres-only lateral FROM pattern.

Optional **views** that wrap double unnest (e.g. `building_goods`, `production_method_goods`) are fine for agents who want a fake junction table — same data, not a second store.

Examples of good array columns: active PMs, `short_inputs`, PM-group ids on a building type, recipe/IO `List<Struct{…}>`.

### `buildings`

Per-building modeled economy (from `BuildingEconomics` / `WorldBuilding`).

| Column | Notes |
| --- | --- |
| `building_id` | **key** |
| `state_id` | FK → states |
| `type_id` | Script building type (`building_rye_farm`, …); FK → `building_types` |
| `type_name` | Localized when available |
| `level` / `staffing` | Levels vs staffed levels |
| `employees` | Summary or join to `building_staffing` TVF |
| `profit` / `revenue` / `cost` | As modeled |
| `production_methods` | `TEXT[]` — active PM script ids (`WorldBuilding.production_methods`) |
| `short_inputs` | `TEXT[]` — scarce input good ids (`BuildingEconomics.short_inputs`) |
| `input_goods` / `output_goods` | `List<Struct{good, good_name, qty}>` — script id (**not** raw `GoodIdx`), localized label, quantity. Project from resolved IO / `goods_io`. See nested-column unnest notes. |

Filter examples without a junction table:

```sql
-- Buildings whose active PMs include a given method
SELECT building_id, type_id
FROM buildings
WHERE array_has(production_methods, 'pm_steam_engines');  -- exact DF fn name TBD; document chosen helper

-- Buildings short on tools
SELECT building_id FROM buildings
WHERE array_has(short_inputs, 'tools');
```

**When to add a child table anyway:** if the dominant query is “all buildings that consume good X with qty” and List-contains / UNNEST pushdown is weak, expose a **view** `building_goods` that UNNESTs `input_goods`/`output_goods` (or a materialised provider over the same vecs). Prefer view-from-array over a separate inventing HashMap of edges.

### Defs: `building_types`, `production_methods`

Static catalog from `GameDefs` (same for all saves once defs are loaded). Prefer these for “what *can* a rye farm make?”; use instance `input_goods`/`output_goods` for “what is this building actually ordering?”

#### `building_types`

| Column | Notes |
| --- | --- |
| `type_id` | Script id; **key** (`BTreeMap` → Exact `=` and range) |
| `type_name` | Localized |
| `group_id` | Building group |
| `city_type` | If present |
| `production_method_groups` | `TEXT[]` — already a `Vec` on `BuildingType`; Exact contains/`array_has` if we bother, else scan+filter |

#### `production_methods`

| Column | Notes |
| --- | --- |
| `pm` | Script id; **key** (`BTreeMap` → Exact `=` and range) |
| `pm_name` | Localized |
| `inputs` / `outputs` | `List<Struct{good, good_name, qty}>` — project `GoodIdx` → script id + `labels`; never expose bare idx as the only key |

```sql
-- Double unnest: list → rows, then struct → columns
SELECT pm, unnest(unnest(outputs))
FROM production_methods;
-- → pm | good | good_name | qty

-- Filter after explode
SELECT pm, good, qty
FROM (
  SELECT pm, unnest(unnest(outputs))
  FROM production_methods
)
WHERE good = 'grain';
```

Optional convenience view `production_method_goods` = that double-unnest pattern — ship in v1 if MCP examples need join-shaped SQL without teaching `unnest(unnest(…))`.

### `pops`

Collapsed state pops from the prices/JSON `state_pops` list (that name is **not** a SQL table).

| Column | Notes |
| --- | --- |
| `state_id` | **key** (part) |
| `profession` | Script id (game/defs often say `pop_type`; SQL column is `profession`) |
| `workforce` / `dependents` | |
| `literacy` | |
| Need basket fields | Or separate `state_needs` table |

### `state_qualifications`

Profession stock vs jobs (from `state_qualifications`).

| Column | Notes |
| --- | --- |
| `state_id` | |
| `profession` | |
| `stock` / `jobs` / `shortage` | Align with alerts staffing vocabulary |

### `countries`

| Column | Notes |
| --- | --- |
| `country_id` | |
| `tag` | **key** |
| `name` | |

### `constructions`

Full private + government construction queues from save IR (`World.constructions`). **Not** the single planning head `queued_building` — one row per order.

| Column | Notes |
| --- | --- |
| `order_id` | Paradox construction id (**key** part with `queue`) |
| `queue` | `private` or `government` |
| `position` | Dense 0-based index within `(country_id, queue)` (scan order) |
| `country_id` | Owner resolved from the order's state; nullable if unknown |
| `state_id` | Target state; nullable if missing on the order |
| `building` | Building type script id; FK → `building_types.type_id` |
| `building_name` | Localized label when defs provide one |
| `remaining` | Remaining construction points when present |

```sql
SELECT queue, position, building, building_name, remaining
FROM constructions
WHERE country_id = 16777216 AND queue = 'government'
ORDER BY position;
```

**Storage:** `Vec<WorldConstruction>` ordered private then government by `order_id`. Exact pushdown on `order_id`, `country_id`, `state_id`, `queue`, `building`.

### `formations` (military)

Conservative military snapshot fields once exposed by the analysis API (manpower, etc.). Exact columns follow the military JSON when wired.

### Optional later

- `building_groups`, `state_needs` as first-class tables
- Multi-save compares via internal `save_id` (advanced; omit from default docs prompts)

## Keys, indexes, and joins

Do not tell agents to `CREATE INDEX`. Speed comes from **provider pushdown** into Rust structures. DataFusion has no “hash index” type — you advertise filter pushdown and implement the lookup yourself.

**Pushdown policy (normative)**

| Backing already in Rust | Equality (`col = ?`) | Range (`<`, `>`, `BETWEEN`, …) | Output ordering |
| --- | --- | --- | --- |
| **BTreeMap** (or other ordered map we already keep) | **Exact** | **Exact** (use `range`) | Yes, when scanning in key order |
| **HashMap** / intern / linear `index_of` / name→id hash built at bind | **Exact** | **Do not push** (Unsupported) — let DataFusion filter after scan if needed | Only if we explicitly sort the scan |

Examples:

- Goods/needs script ids via `goods_order` / `index_of`: Exact on `=`; no range Exact unless we add an ordered id index.
- Localized labels (`labels`, display names) and state **region/name** hash indexes: Exact on `=` only.
- Any existing `BTreeMap` keyed by something we expose as a column (e.g. defs keyed by script id if we treat that map as ordered): Exact on `=` **and** Exact on ranges; declare scan ordering on that key.

**Exact vs Inexact:** Exact means every returned row satisfies the predicate (DF will not re-filter). Prefer Exact for our lookups. Use Inexact only for true over-fetch pruning, not because the structure is a hash map.

**Labels:** Expose **both** non-localized script/region ids and localized display names as separate columns (e.g. `good` + `good_name`, `region_id` / script region key + `region_name`). Index pushdown on whichever side we have a map for; typically script id is denser/stabler for joins, localized name for human/`WHERE` filters (Exact equality only unless a btree exists).

Other rules:

- Prefer joins on `state_id` / `good` / `building_id` (script ids); use localized names in `WHERE`.
- Declare **output ordering** when a scan walks a btree or a vec sorted by id so sort-merge can apply.
- Building a **HashMap** name→id at `use_save` for states is fine (states have no btree today); that enables Exact `=` only, not range.
- List/`array_has` filters: Exact pushdown only if we implement contains against the in-memory vec (cheap for small lists); otherwise Unsupported and DF filters after projection.

## Scalar functions

| Function | Returns | Notes |
| --- | --- | --- |
| `good_price(good TEXT)` | FLOAT | Active-session market price; NULL if unknown good |
| `army_power()` | FLOAT | Player country's `army_power_projection` when known. **NULL** only if the bound world has no `player_tag`. **Errors** (logged) if a player is bound but save IR has no projection fields — never a silent `0` / NULL for “unknown.” |
| `player_tag()` | TEXT | Bound world's played country tag (`World.player_tag`). **NULL** if unset — no first-country fallback. Use with `states.owner_tag` for domestic filters. |

## Table-valued functions (diagnostics)

All read the **active** session unless noted. Column contracts should stay stable (treat as schema).

### `alerts([scope])`

One row per alert from `vic3-prices::alerts` ([`AlertsResult`](json-schema.md)).

- **`alerts()`** (default) — player-scoped: keep rows whose `state_id` is **NULL** or owned by `World.player_tag` (strict tag; no first-country fallback). Foreign-state rows are dropped.
- **`alerts('all')`** — unfiltered full-save set (previous default).
- Any other argument is a plan error. Only the string literal `'all'` is accepted as the one-arg form.

| Column | Type | Notes |
| --- | --- | --- |
| `id` | text | |
| `kind` | text | Enum as in schema (snake_case) |
| `severity` | int | |
| `title` | text | |
| `summary` | text | |
| `state_id` | int nullable | |
| `building_id` | int nullable | |
| `good_id` | text nullable | |
| `evidence` | text | JSON array of `{label,value}` |
| `mitigations` | text | JSON array of mitigation objects (same shape as alerts JSON) |

Nested `staffing` on employment alerts is not inlined here — use `building_staffing(state_id)`.

### `shortage_analysis(good TEXT)`

Expands goods-shortage alerts for one good (`NULL` = all electricity / transportation / goods shortage alerts). Magnitudes come from the market `goods` row; evidence/mitigations are JSON from the alert expander (no invented economics).

| Column | Type | Notes |
| --- | --- | --- |
| `good` | text | Script id |
| `alert_id` | text | |
| `kind` | text | `electricity_shortage` \| `transportation_shortage` \| `goods_shortage` |
| `severity` | int | |
| `title` | text | |
| `summary` | text | |
| `state_id` | int nullable | Hint from short-input buildings when present |
| `building_id` | int nullable | |
| `buy` / `sell` / `shortage` / `price` / `base` | float nullable | Market row; `shortage = max(0, buy − sell)` |
| `evidence` | text | JSON |
| `mitigations` | text | JSON |

### `building_staffing(state_id BIGINT)`

One row per building×profession gap for buildings in that state (same profession arithmetic as employment-alert staffing: employed vs jobs-at-full-level, plus state qualification stock/jobs/shortage).

| Column | Type | Notes |
| --- | --- | --- |
| `building_id` | int | |
| `building_name` | text | Localized type label when available |
| `type_id` | text | |
| `staffing` / `level` | float | |
| `profession_id` | text nullable | Null only when the building has no employee rows |
| `profession_name` | text nullable | |
| `employed_here` / `jobs_here` / `missing_here` | float nullable | |
| `state_jobs` / `state_stock` / `state_shortage` | float nullable | From `state_qualifications` |

## Table-valued functions (planning)

### `plan(goal TEXT)`

Runs existing A\* ([`planning.md`](planning.md)) for the active save. **Explicit invoke only** (can be seconds–much longer).

| Column | Type | Notes |
| --- | --- | --- |
| `step` | int | 0-based order |
| `day` | int or float | Cumulative model days |
| `action` | text | Action kind / verb |
| `detail` | text | Human-readable or compact JSON for args |
| `limitations` | text nullable | Optional per-row or only on step 0 |

Optional args (positional): `max_days` (non-negative int, default `3650`), `label` (text, accepted for [`PlanOpts`](json-schema.md) parity, ignored in the result set).

Goal language: [`dsl.md`](dsl.md).

```sql
SELECT step, day, action, detail
FROM plan('research(tech=nitroglycerin)')
ORDER BY step;
```

`limitations` is populated on **step 0 only** (joined with `"; "`); later steps are NULL.

### `gaps(goal TEXT)`

One row per goal atom for readiness (mirrors gaps CLI/UI).

| Column | Notes |
| --- | --- |
| `predicate` | |
| `status` | `cleared` \| `failing` \| `unknown` (metric missing from save IR — not a measured shortfall; e.g. army PP) |
| `detail` | |

## Example queries

```sql
-- After use_save({ "name": "autosave" }) or selector latest
-- Prefer domestic (player-owned) shortages for advice; omit the owner_tag
-- filter only when you intentionally want world-wide rows.

SELECT s.region_name, g.good, g.shortage, g.price
FROM states s
JOIN goods_by_state g USING (state_id)
WHERE s.owner_tag = player_tag()
  AND g.shortage > 0
ORDER BY g.shortage DESC
LIMIT 20;

SELECT * FROM alerts() WHERE severity = 1;
-- Full-save alerts (not player-scoped):
-- SELECT * FROM alerts('all') WHERE severity = 1;

SELECT * FROM shortage_analysis('engines');

SELECT step, day, action, detail
FROM plan('declare-war(tag=FRA, wargoal=conquer_state, state=alsace)')
ORDER BY step;
```

Pure catalog (no session):

```sql
SELECT name, kind, mtime FROM saves ORDER BY mtime DESC LIMIT 5;
```

## Advanced Query UI (native)

The Tauri **Advanced Query** tab uses this same dialect:

- SQL editor → same engine as MCP `query` (`sql_query` invoke → `vic3-sql`)
- Results grid; clicking cells with recognized keys navigates to companion panes (`state_id` / `building_id` → States, `good` / `good_id` → Prices, plan `step` → Timeline highlight)
- In-app docs panel renders this document (and a UDF index) via `sql_docs` — same markdown MCP serves as `vic3://docs/sql`

## Open questions for review

1. **Shortage formula (v1 locked):** `goods.shortage` / `goods_by_state.shortage` = `max(0, buy − sell)` (unmet demand volume). Not Paradox’s shortage flag.
2. ~~Mitigations as JSON columns vs child TVFs.~~ **Locked:** `evidence` / `mitigations` JSON text on `alerts()` / `shortage_analysis()`; staffing via `building_staffing(state_id)`.
3. ~~Whether unqualified names require `use_save` or may fall back to `latest.*` automatically.~~ **Locked:** unqualified ≡ `active.*`; require `use_save` / `bind` (`SqlError::Unbound`). No auto-fallback to `latest.*`.
4. Military `formations` column list (wait for stable military JSON).
5. Ambiguous `states.region_name` / region labels: return all rows vs error vs prefer player-owned.
6. Which DF array helpers we document for `TEXT[]` contains (`array_has` vs `array_has_any` vs custom UDF) — IO element type is locked: `List<Struct{good, good_name, qty}>` + `unnest(unnest(…))`.
7. Whether convenience UNNEST views (`building_goods`, `production_method_goods`) ship in v1 or stay doc-only patterns.

## Deferred (Wave 3+)

- `formations` military table
- Optional convenience UNNEST views

## Implementation notes (non-normative)

- Crate: `vic3-sql` registers providers on a `SessionContext` over an in-memory `SessionBinding` (`GameDefs` + `World` + `PricesResult`). Hosts hold the engine next to `vic3-api` session state and call Rust `SqlEngine::use_save` (never a mutating `SELECT`); `saves` reads `vic3-catalog`; `latest.*` loads via `vic3-api` without installing the active session.
- Result shaping: Advanced Query uses `vic3_sql::batches_to_json` (`columns` / `rows` / `row_count`). MCP `query` uses the same JSON shape (formatter currently lives in `vic3-mcp`; keep aligned). `vic3://schema` → `schema_catalog_json()` (facts + diagnostics/planning TVFs + scalars).
- Diagnostics: `alerts()` (player-scoped) / `alerts('all')`, `shortage_analysis(good)`, `building_staffing(state_id)`, `good_price` / `army_power` / `player_tag` wrap `vic3-prices` alerts + market rows + session identity. TVF args must be plan-time literals (`NULL` allowed for `shortage_analysis`).
- Planning TVFs: `plan(goal [, max_days [, label]])` and `gaps(goal)` call `vic3-plan` / `vic3-goals` against the bound snapshot. `label` is accepted for [`PlanOpts`](json-schema.md) parity and ignored in the result set. `limitations` is emitted on step 0 only.
- Pages/wasm continues without this engine in v1.
