# SQL interface (design spec)

**Status:** Wave 0 design for review. Not implemented yet.  
**Engine (planned):** Apache DataFusion.  
**Consumers:** MCP `query` tool, Tauri Advanced Query tab, optional `vic3-analyzer sql` debug.  
**Related:** [`mcp.md`](mcp.md) (agent transport), [`json-schema.md`](json-schema.md) (JSON field names), [`dsl.md`](dsl.md) / [`planning.md`](planning.md) (goals / A\*).

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
| `region_name` | Localized display label when available |
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

**Arrays stand in for junction tables.** DataFusion can explode lists into rows with `unnest` / `UNNEST` (and `unnest_outer` when empty/NULL parents should keep a row). That is the join-shaped path when you need one row per element:

```sql
-- SELECT-list unnest (supported; correlates with the parent row)
SELECT building_id, unnest(production_methods) AS pm
FROM buildings;

SELECT pm, unnest(outputs) AS out   -- struct fields → columns, or list of structs
FROM production_methods;
```

**Caveat (DataFusion today):** `FROM parent p, UNNEST(p.col)` lateral style is **not** supported yet — use `unnest(col)` in the **SELECT** list (or a view that does that). Document the working form in examples; do not teach the Postgres-only lateral FROM pattern.

Optional **views** that wrap the SELECT-list unnest (e.g. `building_production_methods`) are fine for agents who want a fake junction table — same data, not a second store.

Examples of good array columns: active PMs, `short_inputs`, PM-group ids on a building type, recipe input/output lists on a PM row.

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
| `input_goods` / `output_goods` | List of `{ good, qty }` (or parallel arrays) from resolved IO — same as `goods_io` / saved IO vecs |

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
| `inputs` / `outputs` | List of `{ good, qty }` from `ProductionMethod.inputs` / `outputs` (already `Vec<(GoodIdx, f64)>`) |

```sql
-- Defs: PMs that output grain (SELECT-list unnest — not lateral FROM)
SELECT pm, out.good, out.qty
FROM (
  SELECT pm, unnest(outputs) AS out
  FROM production_methods
)
WHERE out.good = 'grain';
```

Optional convenience view `production_method_goods` = that unnest pattern — only if MCP/docs examples want join-shaped SQL; not required if arrays + `unnest` are enough.

### `pops` (or `state_pops`)

Collapsed state pops from `state_pops`.

| Column | Notes |
| --- | --- |
| `state_id` | **key** (part) |
| `pop_type` / `profession` | As modeled |
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
| `good_price(good TEXT)` | FLOAT | Active-session market price |
| `army_power()` | FLOAT | If military snapshot available; else NULL + limitation |

Additional scalars may be added; list them here before shipping.

## Table-valued functions (diagnostics)

All read the **active** session unless noted. Column contracts should stay stable (treat as schema).

### `alerts()`

One row per alert from `vic3-prices::alerts` ([`AlertsResult`](json-schema.md)).

| Column | Type | Notes |
| --- | --- | --- |
| `id` | text | |
| `kind` | text | Enum as in schema |
| `severity` | int | |
| `title` | text | |
| `summary` | text | |
| `state_id` | int nullable | |
| `building_id` | int nullable | |
| `good_id` | text nullable | |

Mitigations / staffing arrays: either JSON text columns or separate TVFs (`alert_mitigations(alert_id)`, `building_staffing(state_id)`) — pick one in implementation and update this doc.

### `shortage_analysis(good TEXT)`

Expands shortage diagnosis for one good (or document `NULL` = all scarce goods). Columns TBD to match expander evidence (state_id, building_id, magnitudes, suggested mitigations). Must not invent economics beyond existing alert/what-if helpers.

### `building_staffing(state_id BIGINT)`

Per-building profession gaps for a state (from employment / qualification alerts).

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

Optional args (DataFusion named args or extra parameters): `max_days`, label — mirror [`PlanOpts`](json-schema.md).

Goal language: [`dsl.md`](dsl.md).

```sql
SELECT step, day, action, detail
FROM plan('research(tech=nitroglycerin)')
ORDER BY step;
```

### `gaps(goal TEXT)`

One row per predicate still failing / cleared for goal readiness (mirrors gaps CLI/UI).

| Column | Notes |
| --- | --- |
| `predicate` | |
| `status` | e.g. `failing` \| `cleared` |
| `detail` | |

## Example queries

```sql
-- After use_save({ "name": "autosave" }) or selector latest

SELECT s.name, g.good, g.shortage, g.price
FROM states s
JOIN goods_by_state g USING (state_id)
WHERE g.shortage > 0
ORDER BY g.shortage DESC
LIMIT 20;

SELECT * FROM alerts() WHERE severity = 1;

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

- SQL editor → same engine as MCP `query`
- Results grid; clicking cells with recognized keys navigates to existing panes (`state_id` → States, `good` → Prices, plan `step` → Timeline highlight, etc.)
- In-app docs panel renders this document (and UDF list) — single source with MCP `vic3://docs/sql`

## Open questions for review

1. Exact `shortage` column formula on `goods` / `goods_by_state`.
2. Mitigations as JSON columns vs child TVFs.
3. Whether unqualified names require `use_save` or may fall back to `latest.*` automatically.
4. Military `formations` column list (wait for stable military JSON).
5. Ambiguous `states.region_name` / region labels: return all rows vs error vs prefer player-owned.
6. List element type for IO: struct `{good, qty}` vs parallel `goods[]` + `qtys[]`; which DF array helpers we document (`array_has` vs `array_has_any` vs custom UDF).
7. Whether convenience UNNEST views (`building_goods`, `production_method_goods`) ship in v1 or stay doc-only patterns.

## Implementation notes (non-normative)

- Crate sketch: `vic3-sql` registering providers + UDFs on a `SessionContext` held next to `vic3-api` session state.
- Pages/wasm continues without this engine in v1.
