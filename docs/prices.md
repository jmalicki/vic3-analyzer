# Prices

Inner problem: find relative prices `p` such that **pop consumption in the loop** is consistent with the wiki/game formula. Not an LP. Not “maximize welfare.”

Crate API and solver rationale live in `vic3-prices` rustdoc (`solve`, `preview`, `alerts`, `warm_rel`, MAPI helpers, `LIMITATIONS`). This page is the design narrative; keep the numbered limitations list in sync with that const.

Solved prices feed `PlanningState` (`good_prices` / modeled GDP) and, via `vic3-api`, the CLI/wasm/Tauri JSON surface plus SQL diagnostics and MCP `query` — those hosts do not re-derive the NLS.
## Market formula

With `PRICE_RANGE` from defines (typically `0.75`):

```text
ratio = (buy - sell) / min(buy, sell)     # P4 locks this divisor against defs/wiki
price = base * (1 + PRICE_RANGE * clamp(ratio, -1, +1))
```

**I1:** `buy == sell` ⇒ `price = base` (within ε).  
**I2:** prices stay in `[1 - PRICE_RANGE, 1 + PRICE_RANGE] * base`.  
**I3:** weakly more buy (sell fixed) ⇒ weakly higher price, away from the clamp.

Zero orders: define a documented convention in P4 (do not divide by zero); property tests cover it.

The first [Market Access Price Impact (MAPI)](https://vic3.paradoxwikis.com/Market#Local_prices) milestone blends the solved whole-save market price with each
state's attributed-order price **inside the residual**. Wage pops shop at that
local price (cost of living / buy-package size). Substitution still uses world
sell-order shares, not local ones.

```text
market_access = clamp(infrastructure / infrastructure_usage, 0, 1)
effective_MAPI = 0.75 * market_access
local_price = effective_MAPI * market_price + (1 - effective_MAPI) * state_price
```

Missing infrastructure data defaults to full access. For each candidate world
price vector, each state settles local prices against its own unscaled buy/sell
(pops at `local_price`, buildings and post-1.9 trade frozen). Access then scales
those pop orders into the single whole-save market residual. Full local orders
still determine `state_price`. Building revenue/cost/profit on the report uses
that state's local prices. Trade routes remain unscaled because their endpoints
are absent from the IR. Laws, technologies, incorporation, state traits, overseas
convoy access, and separate market solves are not included yet. The global goods
table displays an order-weighted average of the local prices.

Geography filters (`Our market` default, `Domestic`, `All`) scope both the
global average and the goods-by-state list. They do **not** re-solve prices. A
single-state page is never geography-filtered.

Foreign states show the country’s current flag when defs can select and render a
coat of arms (laws + `flag_definitions`; unsupported triggers are skipped, not
silently replaced). The flag tooltip is the localized country name when present.

## Pop consumption

Pops buy from **need packages** by wealth. Substitution uses `min_supply_share` / `max_supply_share` (**I4**; implemented in `vic3-defs::substitution_shares`). SoL / real income can move requested quantities; that feedback sits inside the equilibrium, not as a frozen demand vector.

Current saves store pop size as `workforce` / `dependents`; both count as full
household members for demand. Fixture/legacy saves may still use `size`,
`size_wa`, and `size_dn`. Workforce and dependents are kept separately on the
state Population tab. Literacy is `literate / workforce`; 1.13 spells that field
`num_literate` (count of literate workers, not a fraction).

Profession indices in `qualifications={ 15 0=… }` follow vanilla
`common/pop_types` filename order (0 = academics). 1.13 states have no
`qualifications=` of their own — the profession tables live in `pop_statistics`
as `population_by_profession`, `population_workforce_by_profession`, and
`population_employable_qualifications`; the flat `employable_qualifications` /
`pop_workforce_by_type` names are still read first for older saves. State
qualifications compare that stock (or the employable subset when present) to
jobs inferred from pops with a `workplace` building id. Monthly qualification *rates*
are still not simulated from `pop_types` scripts; the table omits a rate unless the save
stores one.

Qualification *advice* is not the old farm→mine→workshop→university ladder for every
profession. `alerts` walks `common/pop_types` (who can qualify, literacy/wealth gates)
and production-method employment (which commercial buildings hire those source pops).
It then keeps only levers that apply to this state's mix: if machinists with high
literacy already live here, an engineer shortage names a university and not farms;
an aristocrat shortage names commercial farms (wealthy farmers) and not mines or
universities. Subsistence farms are never a feeder — they absorb leftover peasants
and do not create farmers who can promote. Player-facing copy lives in
[`crates/vic3-prices/advice/qualifications/`](../crates/vic3-prices/advice/qualifications/)
(one YAML file per profession, plus `_defaults.yml` and a `_vanilla_graph.yml` fallback
when the defs blob has no `pop_types`). Employment alerts are **per state**, with
collapsible per-building staffing and per-profession counts of how many more workers
that building needs versus how many the whole state still lacks. Advancement steps
stay on the state's qualification shortage, not on each mill.

Pop `culture` is a numeric index into `cultures.database` (`0={ type=north_german }`).
That table is not `common/cultures` file order. The defs blob already has
English labels keyed by script id (`cultures_l_english.yml`); resolving the
index is what lets the Population tab show "North German" instead of `0`.
Profession names in `pop_types_l_english.yml` are `@academics! $academics_no_icon$`.
`@academics!` is a texticon for `gfx/interface/icons/pops_icons/academics.dds`
(keyed as `pop:academics`); labels expand the `$key$` and drop the marker so the
States/Pops name is "Academics" next to that icon.

Pop **needs** on the Population tab are the same package-ladder + substitution
path as the residual, valued at each pop's **local** prices. They are model
baskets, not a cashflow ledger from the save.

**Wealth 1–99** is relaxed to a continuous variable during NLS, then **rounded** to an integer wealth. This is not ILP.

## Frozen (explicit)

Until a later phase:

- building employment / hire-fire
- wages
- trade-center volumes

Saved building `input_goods` / `output_goods` and post-1.9 signed
`state.trade.goods` allocations are held fixed while pops adjust. Each saved
trade-capacity allocation is multiplied by the good's `traded_quantity` (or the
vanilla default 10) to recover goods volume. Positive state trade is an import
(a local sell order); negative state trade is an export (a local buy order).
Both are attributed to the trade-center state and access-scaled into the
market. Integer saved-good keys are resolved through the deterministic
`common/goods` source order in the definitions blob.
Government and construction goods orders are not yet projected into the solver.

Saved building IO is authoritative because it records actual current weekly
volumes. Only buildings with no saved IO fall back to the sum of their active
production-method recipes. PM fallback throughput is staffed levels:
`levels * clamp(staffing / levels, 0, 1)` (equivalently `staffing` clamped to
`0..levels`), because real saves record `staffing` in level units.

## Empty markets

Nothing in the model forces an order to exist. When the save contributes no pops
and no recognized production methods, every good prices at exactly its base and
the solve reports `converged` with a zero residual — the same output a perfectly
balanced economy gives. `PricesResult.inputs` counts what actually entered the
solve so the two are distinguishable; `goods_with_orders == 0` means the prices
below it carry no information.

The result also carries state metadata (including arable land and infrastructure),
state pops, building type/group definitions, state-attributed orders, and
per-building model economics. The state page groups buildings from those defs,
shows remaining rural capacity and broadly available default placeholders, and
does not claim complete construction eligibility. Building revenue/cost/profit
use saved current IO when present, otherwise PM quantities × staffed levels,
valued at each building's state local price. They are modeled diagnostics, not cashflow
fields read from the save.

## Solver

`min || r - r_formula(orders(r)) ||²` with box bounds on relative prices.

- **Basin** (Levenberg–Marquardt / trust-region-reflective / L-BFGS-B). Default backend: `Vec` until profiling says otherwise. wasm-safe (no BLAS).
- **Successive substitution** `p ← (1-α)p + α P(c(p))` as warm start / fallback inside the same `solve` API.

`SolveOpts.warm_rel` is the previous solve's relative vector (`price / base`, goods-with-base-price order). When its length matches, Basin starts from that vector and skips successive substitution. A length mismatch is ignored (cold start). CLI `mutate` and wasm `loaded_apply_delta` pass the baseline `relative` this way.

**I5:** residual is always reported. If `status = converged` then residual < ε.

## Alerts mitigations

Shortage detectors can attach ranked mitigations (trade-center advice, local
producer expansion, best production-method upgrade, …). Building those levers
used to rescan every world building per alert. When mitigations are enabled,
`alerts_with` / `goods_shortage_alerts` now build a one-shot
`MitigationIndex` (buildings by type / state / good IO, plus memoized PM
candidate sets and best-PM picks) so each alerts pass pays O(buildings) index
cost instead of O(alerts × buildings) rescans. The lean path
(`with_mitigations: false`) skips the index entirely.

## What-if

Apply a delta (e.g. extra building levels) to the frozen building side, then re-solve. Pop consumption re-equilibrates; employment does not.

A [`WorldDelta`](json-schema.md) can also swap production methods on a building id. That clone **clears the building's saved `input_goods` / `output_goods`**, so `goods_io` falls back to the new PM recipes × staffed levels. Extra levels on a type or instance keep saved IO and scale it with the new level. Subsidy toggles are accepted and ignored.

## Limitations

These strings are carried by rustdoc and CLI/wasm JSON. The web UI links here
instead of placing solver diagnostics above the main results:

1. Wealth is relaxed continuous then rounded; not the discrete in-game ladder during the solve.
2. Prices are clamped to ±`PRICE_RANGE`; the clamp is part of the model.
3. Employment, wages, and trade volumes are frozen except explicit what-if deltas.
4. Pops shop at each state's MAPI-blended local prices; state orders are infrastructure-access-scaled into one whole-save market; missing access defaults to 100%, and extra MAPI modifiers and overseas convoy constraints are not modeled.
5. The solve residual is part of the answer; a large residual means the model did not find a consistent pop/price fixed point.

## What-if / prices result shape

See [`json-schema.md`](json-schema.md). Every result includes `limitations: string[]` and `residual`.
