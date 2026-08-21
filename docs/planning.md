# Planning

Search finds a **time-optimal** sequence of goal-relevant moves under our model ([`dsl.md`](dsl.md), [`prices.md`](prices.md)). Optimality is not Paradox’s binary.

## PlanningState

A compact projection of the save + last price solve: date, country tag, techs, laws checkpoints, building levels we can change, construction queue heads, treasury/debt flags needed for `solvent`, interest/infamy, good prices, and whatever atoms the compiled goal reads.

It is **not** the full save. Hash it. **I8:** identical state ⇒ identical hash; applying an action is deterministic.

### Fill rules (`from_save` / `from_world`)

| Field | Source |
| --- | --- |
| `date`, `country` | `meta_data.game_date`; country `definition` / tag |
| `techs` | Country tech fields + top-level `technology` manager (`acquired_technologies`) |
| `laws` | Active law script ids from the law manager for the country |
| `infamy` | Country `infamy` when present and finite |
| `queued_tech` | Country `currently_researching`, else `technology.research_technology` |
| `queued_building` | First private, then government, construction order in owned states (in-flight head) |
| `constructions` | Full private then government queue for the country (`PlanningConstruction` rows) |
| `good_prices`, `gdp` | Last price solve (`gdp` only via `*_with_prices`) |
| `treasury`, `weekly_balance`, `debt_*`, `credit_*`, `solvent` | Country budget |
| `population_weighted_wealth` | Pops in states owned by the country |
| `army_power_projection`, `interest_states` / `interest_regions` | Country `cached_total_army_power_projection` (else army formation `power_projection`); `interest_marker_manager` + country `declared_interests` (normalized for DSL `state=` / `region=`) |
| `building_level_deltas`, `pm_overrides`, `tax_level` | Empty / zero at load; sim branches fill them |
| `queued_interest`, `queued_army_target`, `queued_law` | Empty at load; sim-only in-flight interest / army / law |

`queued_building` remains the single sim wait slot. `constructions` is the full ordered list for exposure (SQL / UI / future goals). Sim `QueueBuildingLevel` pushes a government row and sets the head; `BuildingCompleted` pops the finished row and advances the head to the next entry when the save queue had more.

`from_world` reads the same projected fields off [`WorldCountry`](../crates/vic3-prices/src/world.rs) after `World::from_save`.

## Consumers (same atoms)

Compiled atoms are the contract across surfaces:

| Surface | Entry | Notes |
| --- | --- | --- |
| Web presets | `web/src/planTemplates.ts` | Ordinary DSL strings (`declare-war(state=alsace)`, `gdp >= …`, …); no bypass of compile/eval |
| Gaps / Timeline UI | wasm `loaded_gaps` / `loaded_plan` | Same `PlanOpts` / gaps JSON as CLI |
| SQL | `plan(goal [, max_days [, label]])`, `gaps(goal)` | TVFs compile the goal, project `PlanningState::from_world_with_prices`, then call `vic3-plan` / `vic3-goals` ([`sql.md`](sql.md)) |
| CLI | `vic3-cli gaps` / `plan` | Identical JSON field names ([`json-schema.md`](json-schema.md)) |

## Graph

Nodes: `PlanningState`.  
Edges:

- **Decision** (0 days): queue a tech, start a building, declare interest, expand army power, **switch PM**, **queue a law checkpoint**, **adjust tax** when `weekly_balance` is open, …
- **At most one event-wait** per expansion: wait until tech finishes, a construction slot frees, interest establishes, army expansion completes, a **law** enacts, or payday **only if solvency is an open atom**.

**I6:** event-wait never decreases date. No wait edge if nothing is in flight and solvency is not open.

Successors are **goal-relevant** only (the compiled predicate’s atoms). Do not enumerate the whole game. PM switches and building-level adds are capped (`max_pm_candidates` / `max_pm_overrides` / `max_added_levels_per_type`); tax steps are capped (`max_tax_steps`).

After build/PM edges, re-solve prices (frozen labor/trade still apply).

## A* — locked

Use `rust_advanced_heaps::pathfinding`:

- `SearchNode`: `successors`, `is_goal`, `heuristic` (0 = Dijkstra)
- `shortest_path` with a `DecreaseKeyHeap` (default `PairingHeap`; radix if costs stay integer days)
- `shortest_path_lazy` **only** as a correctness baseline in tests

Do **not** use crates.io `pathfinding`. Do **not** write a third A* loop.

`N: Clone + Eq + Hash` is interned as a HashMap key. **`N` must be cheap** — `Arc<PlanningState>` or a compact hash plus `Arc` to defs/goal. Never put a fat state struct in the intern map key.

Heuristic `h`: admissible estimate of remaining **days**. **I7:** on tiny constructed timed DAGs, `h` never exceeds true remaining cost.

Event-wait and decision edges are just `successors()`. Goal/defs ride on the node (or an `Arc` it holds).

Production `Vic3Node` uses the compiled goal as a dependency DAG relaxation.
Research, interest, raisable army-power, and law atoms contribute their fixed model
durations whenever still open, independent of which item (if any) is queued; AND
takes the maximum child bound (actions may overlap), OR takes the minimum, and
NOT / atoms without a proven timing model (fiscal payday, SoL, tax, …) contribute
zero. Open `good_price` / `gdp` atoms contribute `construction_days` when no
zero-day SwitchPm candidate exists for those atoms; otherwise they contribute
zero so a free PM close stays admissible. Keeping the bound stable across
zero-day queue edges preserves A* consistency when closed nodes are not
reopened. Property tests compare this bound with true remaining costs for
reachable research formulas.

## Non-tech action readiness

Saved-wealth (`population_weighted_wealth`) stays diagnostic until a wage model
exists. Fiscal atoms use a **compact payday model** in `vic3-sim`, not a full
Paradox treasury simulation:

- When `solvent`, `credit_headroom`, or `debt_principal` is an open atom and one
  weekly tick would move that atom closer, successors emit a single payday
  event-wait (`SimConfig::payday_days`, default 7).
- Each payday applies the **frozen** saved `weekly_balance` sample to treasury
  and debt principal (surplus pays principal first, then raises cash; deficit
  spends cash then borrows), then refreshes `credit_headroom` / `solvent`.
- Open `weekly_balance` goals use **AdjustTax**: a zero-day step shifts the
  frozen balance sample by `tax_balance_per_step` (default 50) and records a
  `tax_level` offset capped by `max_tax_steps`. This is not Paradox’s tax UI.
- SoL wealth is unchanged. No interest rate schedule, gold reserve floor,
  credit-limit growth, or investment pool — only the frozen balance vs known
  principal/credit book, plus the compact tax offset.

Declared interest and army power projection both **project from save IR** and
have compact sim actions: queue a goal-relevant interest (`state=` / `region=`)
or army expansion to the open comparison target, then event-wait a fixed model
duration (`interest_days` / `army_expansion_days`, defaults 90 / 180). Completing
interest inserts the id into `interest_states` or `interest_regions`; completing
army sets `army_power_projection` to at least the queued target (aligned with
`DECLARE_WAR_ARMY_THRESHOLD` / `army_power_projection >= 100` compile constants).
These queues share the single in-flight slot with tech, building, and law, so a
declare-war branch that still needs both interest and army pays the sum of the
two waits even though the heuristic AND-bound is their maximum.

**Law checkpoints:** `has_law(…)` reads projected active laws (`law_` prefix
insensitive). Successors queue the missing law then event-wait `law_days`
(default 180). Infamy is projected onto `PlanningState` for later declare-war
extras but is not yet an atom or action.

**Switch PM:** when economy context is present and an open `good_price` / `gdp`
atom can be helped by an alternate production method from defs groups (or
observed peer PMs), emit a capped set of zero-day `SwitchPm` decisions. Applying
one stores a per-building override, clears that building’s saved IO via the
world clone, and re-solves prices. Branching is bounded by
`max_pm_candidates` / `max_pm_overrides`.

The first economy construction action is a building-level decision followed by a
fixed-time construction event and price re-solve. `vic3-prices` preserves the
building's staffing ratio and scales absolute saved IO per level, while leaving
unrelated employment, wages, and trade frozen. `PlanningState` hashes compact
per-type level deltas, PM overrides, tax level, and its single queued
building/law; immutable world/defs/solver inputs ride outside the A* key.
Successors select only producers/consumers relevant to an open `good_price`
atom; increasing GDP goals consider the three highest current output-value
building types. Added levels are capped per type, keeping the search finite.
Construction timing is a model constant, not a claim about Paradox's queue.

**Research innovation capacity** (queued-tech throughput gates beyond a single
queue head) stays **sim-only / undocumented as compile conjuncts** for now —
`research(tech=…)` still compiles to `has_tech` alone.

## P9a vs P9b

- P9a: toy `SearchNode`s, no Vic3, I7 + known shortest path + I8.
- P9b: `SearchNode` for `vic3-sim` successors.
