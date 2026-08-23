# Design: Incremental shop / settle cache for price re-solves

Status: **proposal only** — not implemented. Intended as a follow-up PR after
compact [`equilibrate`](../crates/vic3-prices/src/solve.rs) / [`report`](../crates/vic3-prices/src/report.rs)
split. Captures timing findings and a preferred shape for “hotfix on mutate.”

---

## Problem

A* / PEA* planning re-solves prices on many economy edges. After moving UI/SQL
packaging out of the hot path (`equilibrate` → `SolveOutcome`; `solve` →
`report`), a cold Prussia 1836 equilibrate is still ~**19 ms**, while a full
`solve` (with report) is ~**146 ms**.

`SolveOpts::warm_rel` (Basin warm-start) does **not** help on that path:
cold vs warm equilibrate medians differ by ~1–2% (noise). The NLS is not the
bottleneck.

Most of the remaining ~19 ms is **shop / settle setup** rebuilt from scratch
every call:

- access-scaled non-pop orders
- per-state shops (building + trade frozen buy/sell, frozen pop buy, wage bins)
- `NeedShares` / `UnitBaskets` from the sell mix
- then the residual loop (local settle + Basin)

Planning freezes employment, wages, and trade. Between nodes, only **building
IO** (extra levels, PM overrides, synthetic greenfield rows) usually changes.
Rebuilding all shops from the full building/pop lists is redundant.

Separately, planning still `clone`s `EconomyContext::base_world` and reapplies
deltas on every refresh (~3 ms clone on that save) — related waste.

---

## Idea

Cache the derived shop/settle inputs and **hotfix them when the world is
mutated**, instead of reconstructing from a bare `World` on every
`equilibrate`.

Conceptual flow:

```text
base_world + defs
    → build ShopCache once
apply +1 level / PM swap / …
    → CoW-clone cache, patch touched state(s) / building IO
equilibrate
    → read cache, run NLS only (optional warm_rel still fine, low value)
```

### What to cache

Roughly today’s `state_shops` + market frozen orders + (lazily) unit baskets:

| Piece | Stable under planning freezes? | Hotfix when… |
| --- | --- | --- |
| Pop wage bins / frozen pop buy | Yes | Pops change (rare in planning) |
| Trade orders | Yes | Trade edited |
| Per-state building buy/sell | No | Levels / PMs / building add |
| Access / MAPI | Usually | Infra usage changes (e.g. CS) |
| `NeedShares` / `UnitBaskets` | Maybe | Global sell mix moves enough |

Hotfix for a building edit: subtract old IO from that state’s frozen
buy/sell (and access-scaled market totals), add new IO. Mark baskets dirty if
sell composition changed; rebuild baskets only then (still cheap vs full
`state_shops`).

---

## Where the cache should live

Prefer **not** a deep-copied blob of plain fields on `World`:

1. **Clone tax** — planning clones worlds often; a fat cache must be
   `Arc<ShopCache>` with copy-on-write on hotfix, or cloning stays expensive.
2. **Stale risk** — any path that mutates `buildings` / trade / pops without
   going through cache-aware APIs leaves a silent wrong solve.
3. **Layering** — shops are a *prices* derived view; save IR should not own
   solver internals by default.

Preferred shapes (pick one in the implementing PR):

- **`ShopCache` beside `World`**, owned by `EconomyContext` (or returned from
  mutators). Planning applies deltas by patching the cache instead of
  full rebuild + optional world clone.
- **`Arc<ShopCache>` lightly attached to `World`** (`Option` / once-cell),
  updated only by `with_extra_levels*`, `with_production_methods`, etc., with
  `from_save` building it once or leaving `None` for lazy fill.

Either way: **mutations own the incremental update; `equilibrate` only reads.**

---

## Non-goals / keep separate

- Do not fold UI/`PricesResult` packaging back into the hot path (`report` stays
  off A*).
- Do not rely on `warm_rel` for wall-time wins on current profiles; keep it for
  API parity / tiny residuals if desired.
- Full world clone elimination can ship with the cache PR or immediately after;
  same motivation (delta apply without copying 6k buildings every node).

---

## Suggested validation

Reuse / extend `vic3-prices` `live_timing` (ignored, `VIC3_SAVE` + defs):

1. Cold `equilibrate` baseline (already ~19 ms on Prussia 1836).
2. Same after +1 level / PM swap with cache hotfix — expect setup ≪ rebuild.
3. Parity: goods prices and building revenues within a tight relative tol vs
   full rebuild (planning GDP already tolerates ~1e-9 relative noise).

Unit tests: mutate one building via cache-aware API; assert patched shop orders
match a from-scratch rebuild for that state.

---

## Pointers

- Setup today: [`crates/vic3-prices/src/solve.rs`](../crates/vic3-prices/src/solve.rs)
  (`state_shops`, `access_scaled_non_pop_orders`, `split_pop_buy`).
- Planning apply path:
  [`EconomyContext::apply_planning_to_world`](../crates/vic3-planning/src/sim.rs).
- Methodology overview: [`prices.md`](prices.md).
