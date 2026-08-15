# Invariants

If rustdoc or these docs say “therefore X”, a property test must fail when X is false. Names below are the test names to implement in the listed phase.

| ID | Claim | Phase | Test |
| --- | --- | --- | --- |
| I1 | `buy == sell` ⇒ market price = base (within ε) | P4 | `proptest` on `price(base, buy, sell, defines)` |
| I2 | prices stay in `[1-PRICE_RANGE, 1+PRICE_RANGE] * base` | P4 | `proptest` including clamp extremes |
| I3 | weakly more buy (sell fixed) ⇒ weakly higher price (away from clamp) | P4 | `proptest` |
| I4 | substitution respects `min_supply_share` / `max_supply_share` | P3a / P4 | `proptest` on synthetic need tables |
| I5 | residual always reported; `status=converged` ⇒ residual < ε | P4 / P5 | `proptest` + CLI JSON field |
| I6 | event-wait never decreases date; no wait if nothing in flight and solvency not open | P8 | `proptest` on successor lists |
| I7 | `h` never exceeds true remaining days on tiny constructed graphs | P9a | `proptest` timed DAGs |
| I8 | identical `PlanningState` ⇒ identical hash; actions deterministic | P8 / P9a | hash + apply round-trip |
| I9 | `AnalysisRecord` JSON round-trip; compare(self, self) is empty diff | P11 | serde round-trip + diff fixture |

## Declare-war compilation (P7a)

`declare-war(...)` always includes interest, army, munitions-price, and solvent atoms.

## Determinism

Same IR + defs + opts ⇒ same prices JSON and same plan (I8). Archive fingerprint is a hash of save bytes, not of mtime.
