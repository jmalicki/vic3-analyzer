<!-- Shared endmatter for player-facing guides. Include via copy or link. -->

## Model notes

Prices, what-ifs, and plans run **under our model**, not Paradox’s binary. Results include a `limitations` list; see [prices](../prices.md) and [planning](../planning.md) for method details.

- **Frozen labor / trade in solves** — Employment, wages, and trade volumes stay frozen except explicit what-if deltas; pop demand still re-equilibrates in the price loop.
- **Not a full pop / migration planner** — We do not fully simulate inter-state [migration](https://vic3.paradoxwikis.com/Migration), monthly [qualification](https://vic3.paradoxwikis.com/Profession#Qualifications) accrual as the engine of A*, or civilian hire/fire across the whole economy. We **surface** save-backed qualifications, staffing, and literacy so you (and an LLM) can spot bottlenecks; Timeline is not a closed-loop pop sim.
- **MAPI / market access simplified** — Local prices use infrastructure-oriented [market access](https://vic3.paradoxwikis.com/Infrastructure#Market_access) and base [Market Access Price Impact (MAPI)](https://vic3.paradoxwikis.com/Market#Local_prices) assumptions; extra modifiers, overseas nuance, and separate market solves are incomplete vs the full game.
- **Planner coverage is uneven** — Some goals close with `plan()` / Timeline (research, modeled GDP, army power projection staffing, solvency when the payday model applies). Others are **gaps-only** today (for example SoL / weekly income presets). War readiness is often a checklist first.
- **Government / construction demand** — Not fully projected into every solve path (see [prices](../prices.md)).
- **Building cashflow** in the UI is model IO, not a claim of bit-identical save ledger rows.
