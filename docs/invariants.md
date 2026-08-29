# Property-Tested Formal Invariants

The `vic3-analyzer` codebase enforces mathematical and behavioral invariants via property-based testing (`proptest`).

If any underlying assumption is violated, the corresponding property test suite fails.

---

## Invariants Registry

| ID | Formal Claim | Test Suite / Coverage |
| :--- | :--- | :--- |
| **I1** | $\text{buy} = \text{sell} \implies \text{price} = \text{base}$ (within $\varepsilon$) | `proptest` on `vic3_prices::solve` with symmetric order vectors |
| **I2** | Prices are bounded in $[(1 - r) \cdot \text{base}, (1 + r) \cdot \text{base}]$ (where $r = \text{PRICE-RANGE}$) | `proptest` covering extreme order imbalances and clamp edges |
| **I3** | Monotonicity: Weakly more buy orders (sell fixed) $\implies$ weakly higher price | `proptest` on price response curves away from clamps |
| **I4** | Pop need substitution strictly respects `min_supply_share` and `max_supply_share` | `proptest` on synthetic pop need packages in `vic3-defs` |
| **I5** | Solver residual is always reported. $\text{status} = \text{converged} \implies \text{residual} < \varepsilon$ | `proptest` across diverse world economic states |
| **I6** | Search transitions strictly advance game time ($\Delta t > 0$). No empty wait edges | `proptest` on `vic3_planning` successor generators |
| **I7** | Remaining-time heuristic $h$ is admissible ($h \le \text{true remaining days}$) | `proptest` on timed dependency DAG relaxations |
| **I8** | State hashing and transition operators are strictly deterministic | Hash and action application round-trip tests on `PlanningState` |
| **I9** | `AnalysisRecord` serde round-trips losslessly. Diffing identical records yields an empty diff | Serde property round-trip tests and diff fixtures |

**I5 note:** Under the unclipped target relative price (\(\tau\)) + box stationarity map ([`prices-equilibrium.md`](prices-equilibrium.md)), shortages on a price bound can leave \(\|R\|\) large while Basin’s projected-gradient termination is still valid. I5’s residual-\(\varepsilon\) gate matches the **current** clipped nested path. Expect a documented revision when the Joint solver becomes default.

---

## Goal Compilation Guarantees

- **`declare-war` Expansion:** `declare-war(...)` compilation is guaranteed to produce `interest_in`, `army_power_projection`, munitions price stability, and `solvent` simple subgoals.
- **`colonize` Expansion:** `colonize(...)` compilation is guaranteed to produce tech prerequisites, colonial laws, naval and army thresholds, and `solvent` simple subgoals.
- **Determinism:** Identical save IR, game definitions, and solve options produce identical price vectors and action plans. Archive fingerprints are cryptographic SHA-256 hashes of save bytes (independent of filesystem modification times).
