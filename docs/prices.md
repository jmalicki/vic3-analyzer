# Price Equilibrium & Economic Modeling Methodology

This document specifies the economic modeling formulation and non-linear equilibrium solver in [`vic3-prices`](../crates/vic3-prices).

The core objective is to find a relative price vector $\mathbf{p}$ such that **pop consumption, substitution shares, and state-level MAPI access** are mutually consistent with game formulas.

Solved prices feed the [`PlanningState`](planning.md) (`good_prices`, modeled GDP), the CLI, Web UI, Desktop GUI, and SQL diagnostics.

---

## Market Price Formula

With `PRICE_RANGE` from game defines (typically `0.75`):

```text
ratio = (buy - sell) / min(buy, sell)
price = base * (1 + PRICE_RANGE * clamp(ratio, -1, +1))
```

### Formal Invariants
- **I1 (Equilibrium at Parity):** $\text{buy} = \text{sell} \implies \text{price} = \text{base}$ (within solver tolerance $\varepsilon$).
- **I2 (Box Bounds):** All prices remain bounded in $[(1 - r) \cdot \text{base}, (1 + r) \cdot \text{base}]$ (where $r = \text{PRICE-RANGE}$, typically $0.75$).
- **I3 (Monotonicity):** Weakly higher buy orders (with fixed sell orders) result in weakly higher prices away from clamp boundaries.

When orders are zero for a good, price defaults to base price without division by zero.

---

## Market Access Price Index (MAPI)

The solver calculates per-state local prices by blending the global market price with state-attributed local orders inside the residual:

```text
market_access  = clamp(infrastructure / infrastructure_usage, 0, 1)
effective_MAPI = 0.75 * market_access
local_price    = effective_MAPI * market_price + (1 - effective_MAPI) * state_price
```

1. **Pop Purchases:** Wage pops purchase need packages evaluated at their **local** blended price.
2. **Access Scaling:** Local pop orders are access-scaled into the national market residual.
3. **Building Economics:** Building revenue, input costs, and profitability are evaluated using each state's local prices.
4. **Geography Filtering:** Geography views (`Our Market`, `Domestic`, `All`) scope visual tables and volume-weighted averages without altering the underlying solve.

---

## Pop Consumption & Need Packages

- **Need Packages:** Pops purchase from wealth-stratified need packages. Substitution across eligible goods follows defined minimum and maximum supply shares (**I4**).
- **Population Units:** Pop sizes are read as `workforce` and `dependents`. Both are counted for household consumption.
- **Continuous Wealth Relaxation:** Wealth levels (1–99) are relaxed to continuous variables during the non-linear solve and rounded to integer levels upon convergence.

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

1. **Frozen Variables:** Building employment, base wages, and trade-center route volumes are held fixed from the save during pop re-equilibration unless explicitly modified by a what-if delta or production method override.
2. **Authoritative Saved IO:** Saved building input/output volumes are authoritative. Buildings with no saved IO fall back to PM recipe quantities scaled by staffed levels.
3. **Empty Markets:** If a save has no orders for a good, it prices at base price. `PricesResult.inputs` tracks active order counts so diagnostics can distinguish an empty market from a balanced one.

---

## Solver Mechanics

The solver minimizes the non-linear residual:
$$\min_{\mathbf{p}} \|\mathbf{p} - \mathbf{p}_{\text{formula}}(\text{orders}(\mathbf{p}))\|^2$$
subject to box bounds on relative prices.

- **Solver Algorithms:** Levenberg–Marquardt trust-region reflective optimization ([Basin](https://crates.io/crates/basin)) with a successive substitution warm start fallback.
- **Warm Starts:** Previous relative price vectors (`SolveOpts.warm_rel`) are reused across what-if previews and delta evaluations to accelerate convergence.
- **Convergence Invariant (I5):** The residual is always reported. If `status = converged`, the residual is guaranteed to be $< \varepsilon$.

---

## Explicit Model Limitations

Every analytical result carries structured limitation strings:
1. Wealth is relaxed to a continuous variable during the solve, then rounded.
2. Prices are strictly clamped to $\pm \text{PRICE-RANGE}$ (typically $\pm 75\%$) as defined by the game.
3. Employment, wages, and trade route volumes are frozen unless explicitly modified.
4. State orders are infrastructure-access scaled into a single whole-save market; overseas convoy limits and separate custom unions are not yet modeled.
5. The solve residual is part of the answer: a large residual indicates the model did not converge to a fixed point.
