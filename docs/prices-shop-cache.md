# Design: Incremental shop / settle cache for price re-solves

Status: **implemented** in `vic3-prices` (`ShopCache`) and wired through
planning apply (`EconomyContext::shop_cache` + `shops_for_planning` →
`equilibrate_cached`).

---

## Problem

A* / PEA* planning re-solves prices on many economy edges. After moving UI/SQL
packaging out of the hot path (`equilibrate` → `SolveOutcome`; `solve` →
`report`), a cold Prussia 1836 equilibrate is still ~**19 ms**, while a full
`solve` (with report) is ~**146 ms**.

`SolveOpts::warm_rel` (Basin warm-start) does **not** help on that path:
cold vs warm equilibrate medians differ by ~1–2% (noise). The NLS is not the
bottleneck.

Most of the remaining ~19 ms was **shop / settle setup** rebuilt from scratch
every call:

- access-scaled non-pop orders
- per-state shops (building + trade frozen buy/sell, frozen pop buy, wage bins)
- `NeedShares` / `UnitBaskets` from the sell mix
- then the residual loop (local settle + Basin)

Planning freezes employment, wages, and trade. Between nodes, only **building
IO** (extra levels, PM overrides, synthetic greenfield rows) usually changes.
Rebuilding all shops from the full building/pop lists is redundant.

---

## Approach (shipped)

```text
EconomyContext::new
    → ShopCache::from_world(base) once, Arc-shared

apply (SwitchPm / BuildingCompleted / …)
    → shops_for_planning: clone baseline, patch deltas
    → equilibrate_cached → write prices + GDP on PlanningState

Search / A*
    → only state transitions; never sees ShopCache
```

| Type / API | Role |
| --- | --- |
| [`ShopCache`](../crates/vic3-prices/src/shop_cache.rs) | Derived shops + market orders + building IO |
| `ShopCache::from_world` | Cold full rebuild (also used by `equilibrate`) |
| `ShopCache::patch_building_io` / `set_building_io` | Hotfix one building’s IO |
| `equilibrate_cached` | NLS + revenues from a cache |
| `EconomyContext::shops_for_planning` | Replay level/PM deltas onto baseline |

Invariant: patched shops match `from_world(apply_planning_to_world(state))`
(unit tests in `shop_cache` and `sim`).

Placement / construction capacity still uses `apply_planning_to_world` (full
world projection). Price refresh does not.

---

## Non-goals / keep separate

- Do not fold UI/`PricesResult` packaging back into the hot path (`report` stays
  off A*).
- Do not rely on `warm_rel` for wall-time wins on current profiles; keep it for
  API parity / tiny residuals if desired.
- Infra / trade / pop edits still need a full rebuild (or future patches).

---

## Validation

- Unit: `patch_building_io_matches_rebuild`, `shops_for_planning_matches_from_world_projection`.
- Live: reuse `vic3-prices` `live_timing` (`VIC3_SAVE` + defs) for cold vs patched
  setup cost on a real save.

---

## Pointers

- Cache: [`crates/vic3-prices/src/shop_cache.rs`](../crates/vic3-prices/src/shop_cache.rs)
- Solve: [`crates/vic3-prices/src/solve.rs`](../crates/vic3-prices/src/solve.rs)
- Planning wire: [`EconomyContext`](../crates/vic3-planning/src/sim.rs)
- Methodology overview: [`prices.md`](prices.md).
