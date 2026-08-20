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
| `queued_tech` | Country `currently_researching`, else `technology.research_technology` |
| `queued_building` | First private, then government, construction order in owned states |
| `good_prices`, `gdp` | Last price solve (`gdp` only via `*_with_prices`) |
| `treasury`, `weekly_balance`, `debt_*`, `credit_*`, `solvent` | Country budget |
| `population_weighted_wealth` | Pops in states owned by the country |
| `army_power_projection`, `interest_states` / `interest_regions` | Country `cached_total_army_power_projection` (else army formation `power_projection`); `interest_marker_manager` + country `declared_interests` (normalized for DSL `state=` / `region=`) |
| `building_level_deltas` | Empty at load; sim branches fill deltas |
| `queued_interest`, `queued_army_target` | Empty at load; sim-only in-flight interest / army expansion |

`from_world` reads the same projected fields off [`WorldCountry`](../crates/vic3-prices/src/world.rs) after `World::from_save`.

## Graph

Nodes: `PlanningState`.  
Edges:

- **Decision** (0 days): queue a tech, start a building, declare interest, expand army power, switch PM, enact a law checkpoint, adjust a tax that the goal cares about, …
- **At most one event-wait** per expansion: wait until tech finishes, a construction slot frees, interest establishes, army expansion completes, a law checkpoint, or payday **only if solvency is an open atom**.

**I6:** event-wait never decreases date. No wait edge if nothing is in flight and solvency is not open.

Successors are **goal-relevant** only (the compiled predicate’s atoms). Do not enumerate the whole game.

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
Research, interest, and raisable army-power atoms contribute their fixed model
durations whenever still open, independent of which item (if any) is queued; AND
takes the maximum child bound (actions may overlap), OR takes the minimum, and
NOT / atoms without a proven timing model (construction, munitions price,
fiscal, SoL, …) contribute zero. Keeping the bound stable across zero-day queue
edges preserves A* consistency when closed nodes are not reopened.
Property tests compare this bound with true remaining costs for reachable
research formulas.

## Non-tech action readiness

Snapshot fiscal (`weekly_balance`, `credit_headroom`, `solvent`) and saved-wealth
atoms are diagnostic-only until a budget/debt or wage transition model exists.

Declared interest and army power projection both **project from save IR** and
have compact sim actions: queue a goal-relevant interest (`state=` / `region=`)
or army expansion to the open comparison target, then event-wait a fixed model
duration (`interest_days` / `army_expansion_days`, defaults 90 / 180). Completing
interest inserts the id into `interest_states` or `interest_regions`; completing
army sets `army_power_projection` to at least the queued target (aligned with
`DECLARE_WAR_ARMY_THRESHOLD` / `army_power_projection >= 100` compile constants).
These queues share the single in-flight slot with tech and building, so a
declare-war branch that still needs both interest and army pays the sum of the
two waits even though the heuristic AND-bound is their maximum.

The first economy action is a building-level decision followed by a fixed-time
construction event and price re-solve. `vic3-prices` preserves the building's
staffing ratio and scales absolute saved IO per level, while leaving unrelated
employment, wages, and trade frozen. `PlanningState` hashes compact per-type
level deltas and its single queued building; immutable world/defs/solver inputs
ride outside the A* key. Successors select only producers/consumers relevant to
an open `good_price` atom; increasing GDP goals consider the three highest
current output-value building types. Added levels are capped per type, keeping
the search finite. Construction timing is a model constant, not a claim about
Paradox's queue.

## P9a vs P9b

- P9a: toy `SearchNode`s, no Vic3, I7 + known shortest path + I8.
- P9b: `SearchNode` for `vic3-sim` successors.
