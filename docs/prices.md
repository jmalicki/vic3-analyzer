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

## Pop consumption

Pops buy from **need packages** by wealth. Substitution uses `min_supply_share` / `max_supply_share` (**I4**). SoL / real income can move requested quantities; that feedback sits inside the equilibrium, not as a frozen demand vector.

**Wealth 1–99** is relaxed to a continuous variable during NLS, then **rounded** to an integer wealth. This is not ILP.

## Frozen (explicit)

Until a later phase:

- building employment / hire-fire
- wages
- trade-center volumes (except what-if deltas on buildings/PMs/levels)

Building PM orders and directed trade-route volumes are reconstructed from the
IR and held fixed while pops adjust. Government and construction goods orders
are not yet projected into the solver.

A building runs one active production method per PM group, so its orders are the
sum over every method the save lists for it (`production_methods`, or the
singular `production_method` a hand-written fixture may use). Methods absent from
the definitions contribute nothing.

## Empty markets

Nothing in the model forces an order to exist. When the save contributes no pops
and no recognized production methods, every good prices at exactly its base and
the solve reports `converged` with a zero residual — the same output a perfectly
balanced economy gives. `PricesResult.inputs` counts what actually entered the
solve so the two are distinguishable; `goods_with_orders == 0` means the prices
below it carry no information.

The result also carries state metadata, state-attributed orders, and per-building
model economics. Building revenue/cost/profit are PM quantities ×
`level × staffing` × solved shared price. They are modeled diagnostics, not
cashflow fields read from the save.

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
