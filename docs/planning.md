# Strategic Planning & A* Search Specification

This document details the state representation, graph transition operators, admissible heuristic bounds, and A* search engine in [`vic3-planning`](../crates/vic3-planning).

The planner evaluates goal predicates and discovers time-optimal action sequences (e.g. tech sequencing, law checkpoints, military barracks expansion, production method upgrades, and debt amortization).

---

## PlanningState Projection

A `PlanningState` is a compact, deterministic projection of the save file and the latest economic solve:

| Field | Source / Projection Logic |
| --- | --- |
| `date`, `country` | Game date and country tag from save metadata |
| `techs` | Acquired technology script IDs |
| `laws` | Active law script IDs |
| `infamy` | Current infamy level |
| `queued_tech` | In-flight active research project |
| `queued_building` | In-flight construction head (private or government) |
| `constructions` | Full ordered construction backlog |
| `good_prices`, `gdp` | Solved goods prices and gross output value from the economic model |
| `treasury`, `weekly_balance`, `debt_*`, `credit_*`, `solvent` | Country fiscal balance and credit headroom |
| `population_weighted_wealth` | Pop-weighted average wealth across owned states |
| `army_power_projection`, `navy_power_projection` | Projected military and naval strength from combat formations and staffing |
| `building_level_deltas`, `pm_overrides`, `tax_level` | Sim-only mutations applied along the planning search branch |
| `construction_rate` | National construction capacity (base + Construction Sector levels) |

**Invariants:**
- **Deterministic State Hashing (I8):** Identical planning states produce identical hashes; applying an action is strictly deterministic.
- **Node Compaction:** The search node wraps `Arc<PlanningState>` or a compact hash to minimize memory footprint during priority queue expansions.

---

## Search Graph Operators

Transitions between states consist of **zero-day decisions** and **event-wait edges**:

### 1. Decision Edges (0 Days)
- `QueueTech`: Selects a tech that unlocks prerequisites for open goal atoms.
- `QueueBuildingLevel`: Adds a level of barracks, shipyards, or economic buildings.
- `SwitchPm`: Swaps production methods to boost GDP or relieve a specific goods shortage.
- `QueueLaw`: Begins a law passage checkpoint.
- `QueueInterest`: Declares a strategic interest in a target region.
- `AdjustTax`: Increments or decrements the tax level to meet weekly budget balance goals.

### 2. Event-Wait Edges (Advances Date)
- Advances the game clock to the earliest completion event: tech research finished, building construction complete, military training staffed, law enacted, interest established, or weekly payday debt reduction.
- **Invariant I6:** Event-wait edges strictly advance the game date ($\Delta t > 0$). No wait edge is generated if no action is in flight and solvency is not an open goal atom.

---

## A* Heuristics & Consistency

Search uses priority queue pathfinding (`SearchNode`, `shortest_path`) from `rust-advanced-heaps`:

- **Admissible Heuristic $h$:** Estimates remaining calendar days by relaxing the remaining goal conjuncts into a dependency DAG:
  - Open tech, interest, military training, and law atoms contribute their minimum model durations.
  - Conjunctions (`AND`) take the maximum bound of parallelizable tracks.
  - Disjunctions (`OR`) take the minimum bound across alternatives.
  - Solvency, tax adjustments, and zero-day PM switches contribute zero days where immediate transitions exist.
- **Invariant I7:** On any valid dependency graph, $h$ never overestimates the true remaining days to goal satisfaction.

---

## Non-Tech Action Models

### 1. Compact Payday Model (Fiscal Goals)
- When `solvent`, `credit_headroom`, or `debt_principal` is an open goal atom, successors emit weekly payday waits (7 days).
- Surplus weekly balance pays down principal before accumulating cash; deficits draw down treasury before borrowing.
- Tax adjustments shift the weekly balance sample by discrete increments (`tax_balance_per_step`).

### 2. Military Power Projection
- Power projection is increased by constructing and staffing military infrastructure:
  - **Army:** `building_barracks` construction followed by training time (`army_training_days`).
  - **Navy:** `building_shipyards` and `building_naval_administration` followed by crew hiring (`navy_crew_days`).

### 3. Production Method Switching
- When an economy context is present, the planner evaluates candidate PM switches on relevant industries, applies the override, and triggers an immediate price re-solve.

### 4. Construction capacity (compact model)
- **Capacity** `R` starts from `base_construction_capacity` and rises by `construction_per_cs_level` per Construction Sector level (projected at load; refreshed when a CS level completes on the plan branch).
- **Cost** per queued level uses save `remaining`, else defs `required_construction`, else `default_construction_cost`.
- **Allocation cap** `max_construction_allocation` limits points/day per job; leftover capacity fills later queue entries, so enough capacity yields parallel builds. Wait edges advance to the soonest completion under that split.
- **Building candidates** are type ids for `QueueBuildingLevel` (search branching), not placement UI:
  - **Direct:** defs building types whose default PM IO helps open `good_price` / raising `gdp`, plus barracks/shipyards/naval admin when PP needs levels (hire stays on the military atom arm). First-of-type is allowed; completion inserts a synthetic world row so prices move.
  - **Dominance:** drop a type only when another candidate is strictly better on modeled axes (goal-good benefit per level ↑, construction cost ↓). No top-N ranking. Incomparable types both stay.
  - **Meta:** Construction Sector when any other build candidate already exists (capacity lever, not IO).
  - **Deferred:** state slots / potentials and building unlock techs (`TODO(buildability)`); A* incumbent upper bound via greedy feasible path (`TODO(anytime-ub)`).
- This is still a compact model — not full Paradox construction-goods demand or script cost tables beyond loaded `required_construction`.

---

## Planning Framework Seams

The planning architecture cleanly separates generic search mechanisms from Victoria 3 specific mechanics:

```mermaid
flowchart TD
    Core["Core Planning Layer<br/>(Goal DSL Algebra, Solvers, Resource Tracks, Backlog ETA)"]
    Host["Victoria 3 Domain Layer<br/>(Atoms, Sugar Compilations, Military Formations, PM Edges)"]
    Peripherals["Optional Peripherals<br/>(Embedded Query TVFs, MCP Adapters, Web Facades)"]

    Core --> Host
    Host --> Peripherals
```

- **Resource Tracks & ETA:** Models queues (e.g. construction sectors with aggregate construction points $R$) where job completion time is derived from prefix work divided by throughput rate: $\text{ETA} \approx \lceil \text{work} / R \rceil$.
- **Layered Definitions Merge:** Base constants $\rightarrow$ Extracted game definition blob $\rightarrow$ Optional JSON overlay files.
