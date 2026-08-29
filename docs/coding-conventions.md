# Coding conventions

House style for Victoria 3 Analyzer. Start here when naming fields or Rust
identity types. Other sections may grow over time.

## Entity field naming

Game entities use three roles. Do not overload them.

| Role | Meaning | Examples |
| --- | --- | --- |
| `id` / `*_id` | **Integer** identity | Dense table index (`GoodId`) **or** Paradox save id (`building_id`, `state_id`, `country_id`, `order_id`) |
| `name` / `*_name` | Internal / script key | `building_logging`, `wood`, culture key `prussian`, country tag-class keys |
| `label` / `*_label` | Localized or display string | `Logging`, `Wood`, `Prussian` |

These roles apply **everywhere**: public DTOs, SQL columns, TypeScript types, **and**
private helpers / locals / evidence builders. Do not name a display string
`*_name` or a script key `*_id` “just because it is internal.” `label` is for
UI-facing text (including alert evidence and mitigation titles). `name` is for
lookups, join keys, and action wiring.

### Prefix rule

**Omit the entity prefix when the field lives on a similarly named struct.** The
struct already scopes the noun.

| On this type | Prefer | Not |
| --- | --- | --- |
| `Good` / `GoodPrice` | `id`, `name`, `label` | `good_id`, `good_name`, `good_label` |
| `BuildingTypeInfo` | `id`, `name`, `label` | `building_type_id`, `building_type_name`, … |
| `CountryInfo` | `id`, `name`, `label` | `country_id` (on the same struct), `country_name`, … |
| `ProfessionCount` | `name`, `label` | `profession_name`, `profession_label` |
| `GoodFlow` / `StateGood` | `name`, `label` (the good) | `good_name`, `good_label` |

**Keep the prefix when the field names a different entity** than the struct (or
when several entities share a row and bare `name` would be ambiguous):

| Context | Prefer |
| --- | --- |
| `BuildingEconomics` → its building type | `building_type_id` / `building_type_name` / `building_type_label` |
| `StateInfo` → geographic region | `region_name` / `region_label` (state display stays `label`) |
| SQL / JSON join keys on mixed tables | prefixed columns as needed (`state_id`, `good_name`, …) |
| Field typed `GoodId` on a non-good struct | `good_id: GoodId` |

Same idea for SQL: a dedicated `goods` table may use `name` / `label`. A
`goods_by_state` or building IO list that also has other identities should use
`good_name` / `good_label`. In IO list structs (`List<Struct{good_name, good_label, qty}>`),
`good_label` is always populated at emit via `SessionBinding::good_label` (never NULL in Arrow).

### Rust newtypes

Prefer `*Id` wrappers for dense indices (`GoodId`, `NeedId`, `BuildingTypeId`),
not `*Idx`. On a similarly named struct the field is bare `id: GoodId`. Elsewhere
keep the prefix (`good_id: GoodId`).

### Hard breaks

Public JSON, SQL columns, TypeScript types, and Rust DTOs hard-rename together.
Do **not** keep serde aliases, dual fields, or compatibility shims for the old
dialect.

### Save IR exception

[`vic3-load`](../crates/vic3-load) keeps Paradox wire names at the load
boundary. Map into analyzer `id` / `name` / `label` (or prefixed forms) when
building world/DTO surfaces.

### Out of scope

Catalog entries, MCP client session ids, and other non-game-entity `id`/`name`
pairs are not covered by this dialect.

### Historical dialect (being removed)

Older docs and APIs often used `*_id` for **script strings** and `*_name` for
**localized** labels (for example SQL `type_id` / `type_name`, and
`GoodPrice.id` = script / `.name` = loc). New code must not follow that pattern:
script is always `name` / `*_name`, loc is always `label` / `*_label`, and `id` /
`*_id` is always an integer. Migrate with the checklist below.

## Migration checklist

| Current | New |
| --- | --- |
| `GoodPrice.id` (script) / `.name` (loc) | `name` / `label` (+ `id: GoodId` where dense index is exposed) |
| `StateGood.good_id` / `GoodFlow.good_id` | `name` (script good key on that row) |
| SQL goods table `good` + `good_name` | `name` + `label` (or keep `good_name` / `good_label` on mixed join tables) |
| `BuildingEconomics.id` / `.type_id` | `id` (instance) / `building_type_name` (+ `building_type_id` when dense) |
| `BuildingTypeInfo.id` / `.name` | `name` / `label` (+ `id: BuildingTypeId` when dense) |
| SQL buildings `type_id` / `type_name` | `building_type_name` / `building_type_label` (prefixed: not a building-type table row alone) |
| `profession_id` / `profession_name` on pop/profession DTOs | `name` / `label` on profession-scoped structs. Else `profession_name` / `profession_label` |
| `culture_id` / `culture_name` | same omit-prefix rule |
| `need_id` / `need_name` | same omit-prefix rule (`NeedId` integer stays `need_id` / `id` as appropriate) |
| `region_id` / `region_name` / `state_name` | on `StateInfo`: `region_name` / `region_label` / `label` |
| `CountryInfo.name` / `tag` | `label` / `name` (keep int `id`) |
| `WorldBuilding.building` (script string) | `building_type_id: BuildingTypeId` internally. Resolve type `name` / `label` at JSON/SQL edges |
| Constructions API `building` / `building_name` | `building_type_name` / `building_type_label` |
| Defs entity `Good.id: String` etc. | `name: String` (script). Labels in `GameDefs.labels` or DTO `label` |
| Rust `GoodIdx` / `NeedIdx` | `GoodId` / `NeedId` |
