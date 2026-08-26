# Coding conventions

House style for Victoria 3 Analyzer. Start here when naming fields or Rust
identity types. Other sections may grow over time.

## Entity field naming

Game entities use three suffixes. Do not overload them.

| Suffix | Meaning | Examples |
| --- | --- | --- |
| `*_id` | **Integer** identity | Dense table index (`good_id: GoodId`) **or** Paradox save id (`building_id`, `state_id`, `country_id`, `order_id`) |
| `*_name` | Internal / script key | `building_logging`, `wood`, culture key `prussian`, country tag-class keys |
| `*_label` | Localized or display string | `Logging`, `Wood`, `Prussian` |

### Rust newtypes

Prefer `*Id` wrappers for dense indices (`GoodId`, `NeedId`, `BuildingTypeId`),
not `*Idx`. Field names still use the `*_id` suffix (`good_id: GoodId`).

### Hard breaks

Public JSON, SQL columns, TypeScript types, and Rust DTOs hard-rename together.
Do **not** keep serde aliases, dual fields, or compatibility shims for the old
dialect.

### Save IR exception

[`vic3-load`](../crates/vic3-load) keeps Paradox wire names at the load
boundary. Map into analyzer `*_id` / `*_name` / `*_label` names when building
world/DTO surfaces.

### Out of scope

Catalog entries, MCP client session ids, and other non-game-entity `id`/`name`
pairs are not covered by this dialect.

### Historical dialect (being removed)

Older docs and APIs often used `*_id` for **script strings** and `*_name` for
**localized** labels (for example SQL `type_id` / `type_name`,
`GoodPrice.id` / `.name`). New code must not follow that pattern. Migrate with
the checklist below.

## Migration checklist

| Current | New |
| --- | --- |
| `GoodPrice.id` (script) / `.name` (loc) | `good_name` / `good_label` (+ `good_id: GoodId` where dense index is exposed) |
| `StateGood.good_id` / `GoodFlow.good_id` | `good_name` |
| SQL `good` + `good_name` | `good_name` + `good_label` |
| `BuildingEconomics.id` / `.type_id` | `building_id` / `building_type_name` (+ `building_type_id` when dense) |
| `BuildingTypeInfo.id` / `.name` | `building_type_name` / `building_type_label` |
| SQL `type_id` / `type_name` | `building_type_name` / `building_type_label` |
| `profession_id` / `profession_name` | `profession_name` / `profession_label` |
| `culture_id` / `culture_name` | `culture_name` / `culture_label` |
| `need_id` / `need_name` | `need_name` / `need_label` |
| `region_id` / `region_name` / `state_name` | `region_name` / `region_label` / `state_label` |
| `CountryInfo.name` / `tag` | `country_label` / `country_name` (keep int `country_id` / row `id`) |
| `WorldBuilding.building` (script string) | `building_type_id: BuildingTypeId` internally; resolve `building_type_name` / `_label` at JSON/SQL |
| Constructions API `building` / `building_name` | `building_type_name` / `building_type_label` |
| Defs entity `Good.id: String` etc. | `name: String` (script); labels in `GameDefs.labels` or DTO `*_label` |
| Rust `GoodIdx` / `NeedIdx` | `GoodId` / `NeedId` |
