# Prices

Inner problem: find relative prices `p` such that **pop consumption in the loop** is consistent with the wiki/game formula. Not an LP. Not “maximize welfare.”

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

MAPI is not implemented yet. State drill-down attributes buy/sell orders to the
parsed state but repeats the shared whole-save synthetic market price; the UI
labels that distinction. A future local-price implementation must copy the
game’s market-access blend from defs rather than inventing one.

Geography filters on by-good / by-state views (`Our market` default, `Domestic`,
`All`) only hide attributed state/building rows. They do **not** re-solve prices.

Foreign states show the country’s current flag when defs can select and render a
coat of arms (laws + `flag_definitions`; unsupported triggers are skipped, not
silently replaced). The flag tooltip is the localized country name when present.

## Pop consumption

Pops buy from **need packages** by wealth. Substitution uses `min_supply_share` / `max_supply_share` (**I4**). SoL / real income can move requested quantities; that feedback sits inside the equilibrium, not as a frozen demand vector.

Current saves store pop size as `workforce` / `dependents`; both count as full
household members for demand. Fixture/legacy saves may still use `size`,
`size_wa`, and `size_dn`.

**Wealth 1–99** is relaxed to a continuous variable during NLS, then **rounded** to an integer wealth. This is not ILP.

## Frozen (explicit)

Until a later phase:

- building employment / hire-fire
- wages
- trade-center volumes (except what-if deltas on buildings/PMs/levels)

Saved building `input_goods` / `output_goods` and directed trade-route volumes
are held fixed while pops adjust. Integer saved-good keys are resolved through
the deterministic `common/goods` source order in the definitions blob.
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

The result also carries state metadata, state-attributed orders, and per-building
model economics. Building revenue/cost/profit use saved current IO when present,
otherwise PM quantities × staffed levels, valued at the solved shared price.
They are modeled diagnostics, not cashflow fields read from the save.

## Solver

`min || r - r_formula(orders(r)) ||²` with box bounds on relative prices.

- **Basin** (Levenberg–Marquardt / trust-region-reflective / L-BFGS-B). Default backend: `Vec` until profiling says otherwise. wasm-safe (no BLAS).
- **Successive substitution** `p ← (1-α)p + α P(c(p))` as warm start / fallback inside the same `solve` API.

**I5:** residual is always reported. If `status = converged` then residual < ε.

## What-if

Apply a delta (e.g. extra building levels) to the frozen building side, then re-solve. Pop consumption re-equilibrates; employment does not.

## Limitations

These strings are carried by rustdoc and CLI/wasm JSON. The web UI links here
instead of placing solver diagnostics above the main results:

1. Wealth is relaxed continuous then rounded; not the discrete in-game ladder during the solve.
2. Prices are clamped to ±`PRICE_RANGE`; the clamp is part of the model.
3. Employment, wages, and trade volumes are frozen except explicit what-if deltas.
4. The solve residual is part of the answer; a large residual means the model did not find a consistent pop/price fixed point.

## What-if / prices result shape

See [`json-schema.md`](json-schema.md). Every result includes `limitations: string[]` and `residual`.
