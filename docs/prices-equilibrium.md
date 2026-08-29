# Price equilibrium solver

Canonical formulation for the multi-region price NLS in [`vic3-prices`](../crates/vic3-prices). Product overview, qualification alerts, and limitation strings for callers live in [`prices.md`](prices.md). Incremental shop setup for planning is [`prices-shop-cache.md`](prices-shop-cache.md).

## Implementation status

| | **Current (shipped)** | **Target (this document)** |
| --- | --- | --- |
| Unknowns | Market relative prices \(r\) only | Joint \(x = (r, \{p^{\mathrm{loc}}_s\})\) |
| Locals | Nested successive substitution per residual eval | Local residual block in one stacked system |
| Price map | Clipped [`price`](../crates/vic3-prices/src/formula.rs) | Unclipped \(\tau\); box bounds enforce \(\pm\rho\) |
| Jacobian | Dense FD on \(r\) | **Sparse / arrowhead only** — never a dense \(N\times N\) Joint Jac |
| Market scope | One whole-save price blob | One **market** star (Vic3 market / CU); multi-market later |

Until the Joint path ships and becomes default, treat the Target column as design intent. Code links below point at current modules.

---

## 1. Problem context

The system models a [spatial price equilibrium](https://en.wikipedia.org/wiki/Spatial_price_equilibrium) inspired by Victoria 3 [Market Access Price Impact (MAPI)](https://vic3.paradoxwikis.com/Market#Market_access_price_impact). States (regions) couple through a **market-level** relative price hub \(r\), not through country-level price auxiliaries—countries are not a price-clearing layer in the game economy.

The framework is a [disequilibrium model](https://en.wikipedia.org/wiki/Disequilibrium_(economics)): prices respond to buy/sell imbalance but are capped at \(\pm\rho\) (typically \(0.75\)). Severe shortage hits the regulatory ceiling; orders need not clear ([non-Walrasian](https://en.wikipedia.org/wiki/Walrasian_auction) shortfall).

### Frozen dimensions (today)

During a solve, **building employment, base wages, and trade-center route volumes are frozen** from the save (unless an explicit what-if / `WorldDelta` changes building IO or PMs). Pop demand is **not** frozen—it sits inside the residual. Frozen trade is why separate Vic3 markets are conditionally independent given trade IO and why this codebase can clear **one market star at a time** without country hubs.

---

## 2. Primitives and state space

- **Priced goods:** Set \(G'\) (a subset of all goods \(G\)). For \(g \in G'\), base price \(b_g > 0\) and price-range \(\rho \in (0,1]\) (NLS unknowns). Goods with \(b_g = 0\) are not relative-price unknowns. Throughout this document, \(g\) refers to \(g \in G'\).
- **States (regions):** \(s \in S\). Effective MAPI weight \(m_s = 0.75 \cdot \mathrm{access}_s\) with \(\mathrm{access}_s \in [0,1]\), so typically \(m_s \in [0, 0.75]\). Local price blends toward the **market** price \(p^{\mathrm{mkt}}\), not an average of other locals ([`effective_mapi`](../crates/vic3-prices/src/formula.rs) / [`local_price`](../crates/vic3-prices/src/formula.rs)).
- **Access \(\alpha_s\):** infrastructure access also scales state orders into the market residual (same infra input as MAPI, different role).
- **Frozen volumes:** per state, non-pop buy \(B^{0}_{s,g}\) and sell \(Q_{s,g}\); national frozen non-pop aggregates; non-wage pop buy frozen; wage bins respond to local prices ([`ShopCache`](../crates/vic3-prices/src/shop_cache.rs)).
- **Target joint state:** \(x = (r, \{p^{\mathrm{loc}}_s\})\) with \(r_g \in [1-\rho, 1+\rho]\), \(p^{\mathrm{mkt}}_g = b_g r_g\), locals in \([b_g(1-\rho), b_g(1+\rho)]\).

**Hub = market.** One relative-price vector \(r\) per Vic3 market. Do not introduce country auxiliaries.

---

## 3. Assumptions and economic mechanics

### A. Continuous wealth (code vs ideal)

Pop demand uses [Laspeyres-style](https://en.wikipedia.org/wiki/Price_index#Laspeyres_price_index) continuous wealth then **floor/ceil lerp** of integer packages ([`consumption`](../crates/vic3-prices/src/consumption.rs)). That is continuous in wealth but **not** \(C^1\) at integers; wealth clamps add nonsmooth points. Treating \(D_s\) as smooth and monotone is a **modeling ideal** for analysis, not a claim about the shipped ladder.

### B. Unclipped target \(\tau\) (target map)

Imbalance ratio (see [`market_ratio`](../crates/vic3-prices/src/formula.rs) for zero-order branches: both empty → \(0\); buy-only → \(+1\); sell-only → \(-1\); else \((B-Q)/\min(B,Q)\)):

\[
\tau(B,Q) = 1 + \rho \cdot R(B,Q)
\]

**Target:** omit \(\mathrm{clip}_{[-1,1]}\) on \(R\) inside the residual so shortage pushes \(\tau\) outside \([1-\rho, 1+\rho]\); the solver **box** enforces the cap ([MCP](https://en.wikipedia.org/wiki/Mixed_complementarity_problem) / projected gradients).

**Current:** [`price`](../crates/vic3-prices/src/formula.rs) still applies `.clamp(-1, 1)` on the ratio.

---

## 4. Simultaneous fixed-point formulation (target)

Stack national and local consistency into \(R^{\mathrm{full}}(x)\).

**Buys and Sells:**

\[
B_{s,g}(x) = B^{0}_{s,g} + D_{s,g}(p^{\mathrm{loc}}_s), \quad
B_g(x) = B^{\mathrm{nat,0}}_g + \sum_s \alpha_s \left[D_{s,g}(p^{\mathrm{loc}}_s) - D^0_{s,g}\right]
\]

\[
Q_g = Q^{\mathrm{nat,0}}_g
\]

(where \(B^{\mathrm{nat,0}}_g\) includes frozen state-population buy, while \(Q^{\mathrm{nat,0}}_g\) contains frozen sell orders only. State-building, trade, and frozen-population orders are weighted by \(\alpha_s\), while world-level frozen extras and stateless orders use access 1.0, matching ShopCache).

**Local residual** (relative-price units):

\[
R^{\mathrm{loc}}_{s,g}(x) = \frac{1}{b_g}\Bigl[
p^{\mathrm{loc}}_{s,g}
- \bigl(m_s p^{\mathrm{mkt}}_g(r) + (1-m_s)\, b_g \tau(B_{s,g}, Q_{s,g})\bigr)
\Bigr]
\]

**Market (national) residual:**

\[
R^{\mathrm{nat}}_g(x) = r_g - \tau(B_g(x), Q_g)
\]

Minimize \(\tfrac12 \|R^{\mathrm{full}}(x)\|_2^2\) subject to the box on \(x\). (Note: This is a box-constrained least-squares formulation, not a strict Mixed Complementarity Problem (MCP). Stationarity implies projected gradients involving \((J^{\mathrm{full}})^T R^{\mathrm{full}}\) vanish, which does not necessarily enforce componentwise sign complementarity on \(R^{\mathrm{full}}\) at the bounds.)

**Shipped nested path** instead solves only over \(r\), with locals from inner successive substitution ([`solve.rs`](../crates/vic3-prices/src/solve.rs)).

---

## 5. Bound shortages (Projected-Gradient Stationarity)

With unclipped \(\tau\) and box constraints: at a hard shortage the hub coordinate sits on \(1+\rho\), and the component residual \(R_g\) need not be \(\approx 0\). Under the least-squares objective, projected-gradient stationarity can hold even with large \(\|R\|\). Callers must not treat \(\|R\|\approx 0\) as the only success criterion once that map ships—see Basin termination flags. Today’s **I5** (`converged` \(\Rightarrow\) residual \(<\varepsilon\)) still matches the clipped nested path; it may evolve with the unclipped target ([`invariants.md`](invariants.md)).

---

## 6. Lifting and star topology (sparsity)

Economically, states couple densely through the market. **Lifting** the market relative prices \(r\) as independent unknowns makes state locals **conditionally independent given \(r\)**: wiggling California locals with \(r\) held fixed does not change New York’s local residual. Cross-state effects travel only through the market hub → **arrowhead / star** Jacobian.

That **market** auxiliary is what enables sparse/fast linear algebra. Joint must **never** assemble a dense \(N\times N\) Jacobian (\(N = |G'| + |S'|·|G|\)).

---

## 7. Implementation notes (`basin`)

Target Joint path: Basin trust-region reflective with **sparse** Jacobian (arrowhead pattern; Basin/`faer` sparse types or structured FD on nonzeros / explicit Schur—never dense full \(N\)).

Nested path today: dense FD on \(r\) only ([`solve.rs`](../crates/vic3-prices/src/solve.rs))—legacy until Joint replaces it.

ShopCache remains the frozen residual input and planning patch surface; Joint removes inner local SS, not the cache ([`prices-shop-cache.md`](prices-shop-cache.md)).

---

## 8. Scope and future markets

**This phase:** one market star (current whole-save blob treated as a single market).

**Future:** partition by **game-defined markets** from the IR (customs unions, subject markets, …). With trade frozen, solve each market’s star independently. When trade is endogenous: outer loop updating trade/convoys, or a coupled multi-market NLS with several market hubs + inter-market flow bounds—still structured sparse, still **no country hubs**.
