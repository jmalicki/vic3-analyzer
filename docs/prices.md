# Price Equilibrium & Economic Modeling Methodology

This document is the **product / methodology hub** for prices in [`vic3-prices`](../crates/vic3-prices): why we solve, MAPI and pop consumption at a glance, frozen dimensions, qualification alerts, and caller-facing limitations.

The **NLS formulation** (joint market + local residuals, lifting/star sparsity, unclipped target map, MCP shortages) is specified in [`prices-equilibrium.md`](prices-equilibrium.md). Incremental shop setup for planning re-solves is [`prices-shop-cache.md`](prices-shop-cache.md).

Solved prices feed the [`PlanningState`](planning.md) (`good_prices`, modeled GDP), the CLI, Web UI, Desktop GUI, and SQL diagnostics.

---

## Why Simple Order Tallying Fails

Victoria 3's economy cannot be accurately projected by simply adding or subtracting fixed order quantities. When production changes or buildings expand:
- **Pop demand is dynamic:** Pops shop across wealth-stratified need packages. As relative prices move, pop substitution and purchasing power shift requested quantities.
- **Local prices depend on market access:** State-level prices blend local supply and demand with national market prices based on infrastructure access and [Market Access Price Impact (MAPI)](https://vic3.paradoxwikis.com/Market#Market_access_price_impact).
- **Cascading profitability:** Factory revenue and input costs change simultaneously across upstream and downstream supply chains.

Rather than relying on un-equilibrated estimates, `vic3-analyzer` solves a non-linear system (NLS) for consistent pop demand and market prices. See [`prices-equilibrium.md`](prices-equilibrium.md).

---

## Market Price Formula (shipped)

With `PRICE_RANGE` from game defines (typically `0.75`):

```text
ratio = (buy - sell) / min(buy, sell)
price = base * (1 + PRICE_RANGE * clamp(ratio, -1, +1))
```

The equilibrium design target uses an **unclipped target relative price** (\(\tau\)) with box bounds instead of an interior clamp (see [`prices-equilibrium.md`](prices-equilibrium.md)). Formal invariants **I1–I3** are in [`invariants.md`](invariants.md).

When orders are zero for a good, price defaults to base price without division by zero.

---

## [Market Access Price Impact (MAPI)](https://vic3.paradoxwikis.com/Market#Market_access_price_impact)

```text
market_access  = clamp(infrastructure / infrastructure_usage, 0, 1)
effective_MAPI = 0.75 * market_access
local_price    = effective_MAPI * market_price + (1 - effective_MAPI) * state_price
```

1. **Pop Purchases:** Wage pops purchase need packages evaluated at their **local** blended price.
2. **Access Scaling:** Local pop orders are access-scaled into the market residual.
3. **Building Economics:** Building revenue, input costs, and profitability use each state's local prices.
4. **Geography Filtering:** Geography views (`Our Market`, `Domestic`, `All`) scope UI tables without altering the solve.

Hub prices are **market**-level (Vic3 markets / customs unions), not country-level. See [`prices-equilibrium.md`](prices-equilibrium.md).

---

## Pop Consumption & Need Packages

- **Need Packages:** Pops purchase from wealth-stratified need packages. Substitution across eligible goods follows defined minimum and maximum supply shares (**I4**).
- **Population Units:** Pop sizes are read as `workforce` and `dependents`. Both are counted for household consumption.
- **Continuous Wealth Relaxation:** Wealth levels (1–99) are relaxed to continuous variables during the non-linear solve (Laspeyres + package lerp) and rounded to integer levels upon convergence.

---

## Pop Qualification Advice Graph

Qualification alerts walk vanilla `common/pop_types` (literacy and wealth prerequisites) and production-method employment to identify exact advancement bottlenecks.

Instead of generic promotion advice, the alert engine filters advice specifically to each state's current demographics:
- If a state has high-literacy **Machinists**, an **Engineer** shortage suggests building a **University** rather than commercial farms.
- An **Aristocrat** shortage suggests commercial agriculture (wealthy farmers) rather than mines.
- **Subsistence farms** are excluded as feeder paths because they absorb surplus peasants without providing the wealth needed to promote.

---

## Model Boundaries & Frozen Dimensions

To provide fast, deterministic evaluations for what-if scenarios and search heuristics:

1. **Frozen Variables:** Building employment and base wages are held fixed from the save during pop re-equilibration unless explicitly modified by a what-if delta or production method override. **Trade-center route volumes** remain strictly frozen during pop re-equilibration (they are separate from the what-if delta or PM override exceptions). **Trade is frozen today**. This is part of the pricing model, not only a future footnote.
2. **Authoritative Saved IO:** Saved building input/output volumes are authoritative. Buildings with no saved IO fall back to PM recipe quantities scaled by staffed levels.
3. **Empty Markets:** If a save has no orders for a good, it prices at base price. `PricesResult.inputs` tracks active order counts so diagnostics can distinguish an empty market from a balanced one.

---

## Solver

Formulation, nested vs joint target, lifting/star sparsity, and MCP shortages: **[`prices-equilibrium.md`](prices-equilibrium.md)**.

- **Warm Starts:** Previous relative price vectors (`SolveOpts.warm_rel`) are reused across what-if previews and delta evaluations.
- **Convergence Invariant (I5):** Residual is always reported. Today `status = converged` implies residual \(<\varepsilon\). Bound-MCP may revise that criterion—see [`invariants.md`](invariants.md) and the equilibrium doc.

---

## Explicit Model Limitations

Every analytical result carries structured limitation strings:
1. Wealth is relaxed to a continuous variable during the solve, then rounded.
2. Prices are strictly clamped to \(\pm \text{PRICE-RANGE}\) (typically \(\pm 75\%\)) as defined by the game.
3. Employment, wages, and **trade route volumes are frozen** unless explicitly modified.
4. State orders are infrastructure-access scaled into a single whole-save market. Overseas convoy limits and separate custom unions (multiple game markets) are not yet partitioned in the IR/solver.
5. The solve residual is always reported. A large residual usually means the solver did not reach an interior price fixed point; it can also mean a bound shortage under a future MCP map (see [`prices-equilibrium.md`](prices-equilibrium.md) §5). Interpret residual together with `status` and per-good prices.

---

## Related design notes

- [`prices-equilibrium.md`](prices-equilibrium.md) — NLS formulation (canonical).
- [`prices-shop-cache.md`](prices-shop-cache.md) — incremental `ShopCache` for planning re-solves (baseline + patch → `equilibrate_cached`).
