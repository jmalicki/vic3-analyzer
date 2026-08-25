# Progress-aware planner heuristic

Developer internals for planner search. Related:
[`planning.md`](planning.md), [`planning-search.md`](planning-search.md).

**Status:** ranking / cheap-bag **library** lives in `plan/progress_h.rs`.
It is **not** wired into PEA or `plan()`. Greedy incumbent $U$ is **not** in
this PR (`TODO(anytime-ub)` in [`planning.md`](planning.md)).

Code: `plan/progress_h.rs` only (unwired).

---

## How to read this doc

**Everything through the ranking formulas is written for a GDP simple subgoal**
(`raise gdp to …`). That is the concrete meter we implement and test against
first: gap, build rates, Construction Sector cheap scores.

Search cost is always calendar days for every goal. What changes for other
simple subgoals is only how progress is measured (gap, per-option yield, which
successors matter). That plug-in table lives at the end:
[Other goal types](#other-goal-types-how-the-gdp-story-changes).

Symbols are **introduced where first used**, in Math | Definition | Rust
tables. Later sections reuse earlier symbols without repeating the table.

**Timeline states:** $\mathrm{state}_t$ is the node being expanded (current);
$\mathrm{state}_{t'}$ is the successor after action $a$. Calendar day may be
unchanged when $e(a)=0$. Rust suffixes: `*_curr` / `*_succ`. Reserve “parent”
for plan-tree ancestry only (e.g. explanation `parent_id`), not bag GDP math.

A **simple subgoal** is a compiled goal node with no further goal children
(`Goal::Simple` / `SimpleSubgoal`). See [`planning.md`](planning.md).

---

## Basic rundown (GDP)

| Math | Definition | Rust |
| --- | --- | --- |
| $a$ | A candidate planner action (enqueue build, queue tech, switch PM, …) | `Action` / bag candidate |
| $\mathrm{state}_t$ | Current timeline state — the search node being expanded | `PlanningState` on the domain / `state_curr` |
| $\mathrm{state}_{t'}$ | Successor timeline state after applying $a$ (may share calendar day with $t$ when $e(a)=0$) | apply result / `*_succ` |
| $g$ | A* path cost so far (sum of edge days) | pathfinder path cost |
| $e(a)$ | Edge cost of action $a$ (often $0$ for queue decisions) | `Successor::days` |
| $h_{\mathrm{adm}}$ | Admissible remaining-days lower bound (timing DAG) | `Vic3Node::heuristic` |
| $h$, $h_{\mathrm{rank}}$ | Progress residual-days estimate; **ranking bias**, not a proven admissible bound | `rank_heuristic` / `rank_heuristic_with_gdp_for_rates` |

1. **Cost is calendar days.** Find a shortest path in days until GDP reaches
   the target (`evaluate` true).
2. **A\*** / open-set search use $f = g + h$ with $g$ = days so far and edge costs in days
   (0-day decisions, positive waits). Today’s wired search still uses
   $h_{\mathrm{adm}}$ for Ready keys; $h_{\mathrm{rank}}$ is library-only until
   search wiring.
3. **$h_{\mathrm{rank}}$ only orders.** Progress residual scores are meant to
   rank the open set / candidate bag so search is not blind. They are **not** a
   proven admissible lower bound and do **not** define an incumbent upper
   bound $U$ (greedy $U$ is a later PR).

The same day-cost skeleton applies to other goals; only the progress meter
changes — see [Other goal types](#other-goal-types-how-the-gdp-story-changes).

---

## Progress ranking for GDP

Used to order candidates (library APIs today; search wiring later). New symbols
for the residual-days bias:

| Math | Definition | Rust |
| --- | --- | --- |
| $G$ | GDP residual gap $\max(0,\mathrm{target}-\mathrm{gdp})$ (often via `gdp_for_rates`) | `simple_subgoal_gap` / `meter_gap_curr` |
| $G_{\mathrm{residual}}$ | Gap left after in-flight / immediate credit | — |
| $\Delta_i$ | Predicted GDP gain when option $i$ completes | `simple_subgoal_delta` / cheap GDP delta |
| $T_i$ | Days until that completion | construction ETA helpers |
| $r_i$ | Option rate $\Delta_i / T_i$ ($T_i=0$ ⇒ credit $\Delta_i$ immediately, do not divide) | — |
| $S_{\mathrm{build}}$ | Spare government construction feed slots | `ConstructionSlots` / `max_parallel_construction_jobs` |
| $R^{*}$ | Aggregate GDP/day for still-open work after in-flight credit | `aggregate_rate_curr` (proxy) |
| $R^{*}_{\mathrm{build}}$ | $R^{*}$ from top construction-fed rates × spare build slots | — |

### Gap $G$

$$
G = \max(0,\ \mathrm{target} - \mathrm{gdp})
$$

$G = 0$ ⇒ GDP simple subgoal done.

### Option yield and duration

$$
r_i = \frac{\Delta_i}{T_i}
$$

If $T_i = 0$, credit $\Delta_i$ immediately — do not form an infinite rate.

### Aggregate rate $R^{*}$ and $h_{\mathrm{rank}}$

$R^{*}$ is the GDP/day assumed for **still-open** work after in-flight credit.
It is built from option rates $r_i$ and per-track spare capacity.

**Option kinds** for GDP (each has $\Delta_i$, $T_i$, hence $r_i$):

- Construction (`QueueBuildingLevel`, in-flight government builds)
- Production methods (`SwitchPm` when enabled; prefer immediate $\Delta$ credit)
- Other zero-day GDP moves (immediate $\Delta$ credit)

**Parallelism is per track:**

- $S_{\mathrm{build}}$ = spare government construction feed slots
- Non-build tracks are separate when they contribute GDP (often rare for pure GDP)

For construction-fed residual progress, sort build rates descending
$r_{(1)} \ge r_{(2)} \ge \cdots$ (with replacement if the same type can
repeat):

$$
R^{*}_{\mathrm{build}}
  = \sum_{j=1}^{S_{\mathrm{build}}} r_{(j)}^{\mathrm{build}}
$$

Special cases:

- $S_{\mathrm{build}} = 1$ ⇒ $R^{*}_{\mathrm{build}} = \max_i r_i^{\mathrm{build}}$
- $S_{\mathrm{build}} = 2$ ⇒ best + second-best build rates
- $S_{\mathrm{build}} = 0$ ⇒ do not divide by zero; advance time on the
  in-flight schedule until a slot frees, then recompute

Non-construction GDP progress that can run alongside builds is credited on the
**timeline / residual $G$**, not counted as a construction slot.

### In-flight credit

Jobs already queued (builds, …), and jobs predicted just queued by a 0-day
candidate, deliver their GDP $\Delta$ on an optimistic completion timeline.
Apply that credit to residual gap (or advance simulated time) **before**
dividing by $R^{*}$. Do not also count those jobs as free slots in $S$.

### Heuristic $h$

Units must match $g$:

$$
h = \frac{G_{\mathrm{residual}}}{R^{*}}
\quad\text{(days)}
$$

Never add gap to rate ($G + r_i$). Never use $r_i$ or $R^{*}$ alone as $h$.

Full residual schedule:

1. Apply immediate $\Delta$ from $T_i = 0$ options.
2. Advance in-flight completions on all tracks.
3. Fill spare slots on each track with that track’s best remaining rates.
4. Return days until residual GDP gap is 0.

When only construction remains and $S_{\mathrm{build}} > 0$:

$$
h = \frac{G_{\mathrm{residual}}}{R^{*}_{\mathrm{build}}}
$$

---

## Cheap bag library (GDP)

Bag-order scorers live in `progress_h` as **library APIs**. They are **not**
called from search yet. Bag order is a **bias**, not a true open-set $f$-layer —
see [`planning-search.md`](planning-search.md#known-mixed-f-on-the-open-set-tolerated-for-v1)
when wiring.

New symbols for cheap scoring (quantities on $\mathrm{state}_t$ use
subscript $t$ / Rust `*_curr`):

| Math | Definition | Rust |
| --- | --- | --- |
| $\widetilde{\Delta\mathrm{gdp}}(a)$ | **Cheap** GDP change guesstimate for $a$ (not a full price solve; not a bag key by itself) | `cheap_gdp_delta_guesstimate` |
| $G_t$, $R^{*}_t$, $H_{\mathrm{follow}}$ | Gap, aggregate rate, and follow-on days on $\mathrm{state}_t$ (one bag) | [`BagResidualCurr`](../crates/vic3-planning/src/plan/progress_h.rs) |
| $T_b$, $T_{\mathrm{CS}}$ | Days until ordinary build / Construction Sector completion | construction ETA helpers |
| $C$ | Construction points/day on $\mathrm{state}_t$ | `PlanningState::construction_points_per_day` |
| $\Delta C$ | Points/day from +1 Construction Sector (× government share) | `cheap_construction_sector_points_delta` |
| — | $\mathrm{state}_t$ context for every cheap score in one expand | [`CheapBagCurr`](../crates/vic3-planning/src/plan/progress_h.rs) |
| $\mathrm{score}_{\mathrm{cheap}}(a)$ | **Bag** order key (cheap follow-on) | `cheap_bag_score` |
| $\mathrm{score}_{\mathrm{emit}}(a)$ | Emit-style key $e(a) + h(\mathrm{completed})$ (library; PEA not wired) | `emit_bag_score` |

**Tilde:** $\widetilde{\cdot}$ = cheap bag approximation. Neither cheap nor
emit scorers redefine path cost $g$.

Always $\mathrm{score}(a) = e(a) + \text{(follow-on days after } a\text{)}$.
**Cheap bag** uses approximate follow-on; **emit** (library) uses residual $h$
on a completed-world state the caller supplies.

### Cheap bag (order only)

Ordinary productive build (capacity unchanged; credit
$\widetilde{\Delta\mathrm{gdp}}$):

$$
\mathrm{score}_{\mathrm{cheap}}(b)
  = T_b + \frac{\max\bigl(0,\ G_t - \widetilde{\Delta\mathrm{gdp}}(b)\bigr)}{R^{*}_t}
$$

Construction Sector (guesstimate — construction-unit scale of follow-on on
$\mathrm{state}_t$). CS also moves GDP via construction-goods demand (iron,
wood, …); the cheap key prioritizes throughput shortening of follow-on days
and does not credit that GDP delta (emit’s full residual after speculative
complete does):

$$
\mathrm{score}_{\mathrm{cheap}}(\mathrm{CS})
  = T_{\mathrm{CS}} + \frac{C}{C + \Delta C}\, H_{\mathrm{follow}}
$$

**Deficiencies vs a full emit / later greedy rebuild** (also in code comments on
`cheap_bag_score`): CS scale ≠ actual slots + CS finish day; cheap path omits
CS GDP from construction-goods demand; cheap GDP guesstimate ≠ full price
solve; the delayed / follow-on portion can score **lower (better)** than this
heuristic once slots and prices are real.

### Emit residual (library API)

$$
\mathrm{score}_{\mathrm{emit}}(a) = e(a) + h(\mathrm{completed})
$$

`emit_bag_score` takes a completed `PlanningState` and returns
$e(a) +$ [`rank_heuristic_with_gdp_for_rates`](../crates/vic3-planning/src/plan/progress_h.rs)
on that state’s GDP. PEA speculative complete / `gdp_for_rates` node fields /
emit mismatch warns are **not** in this PR.

Tie-break (when wired): higher $r_i$ for that candidate, then fingerprint.

---

## Worked example (GDP)

$G = 1000$ (GDP shortfall), $r_A = 10$, $r_B = 20$, $S_{\mathrm{build}} = 2$.

- One imagined slot at B-rate: $h \approx 1000 / 20 = 50$
- One imagined slot at A-rate: $h \approx 1000 / 10 = 100$
- Two spare slots at B-rate: $R^{*}_{\mathrm{build}} = 20 + 20 = 40$,
  $h \approx 1000 / 40 = 25$
- Queue A and B together: $+200$ lands at day 10 while A continues; a free
  slot can take another B. After the second enqueue, predicted in-flight must
  include A so $h$ beats “A alone with idle spare capacity.”

---

## Other goal types (how the GDP story changes)

Day cost $g$ and “$h_{\mathrm{rank}}$ is bias only” are **unchanged**. Swap the
progress meter ($G$, $\Delta$, which options feed $R^{*}$) and, where
relevant, GDP-specific cheap formulas.

| Simple subgoal kind | $G$ | $\Delta_i$ (examples) |
| --- | --- | --- |
| **GDP** (body of this doc) | $\max(0,\ \mathrm{target}-\mathrm{gdp})$ | Predicted GDP from build / PM / … |
| `good_price` band | Distance outside $[\mathrm{lo},\mathrm{hi}]$ | Move toward band (or $1$ if enters) |
| Army / navy power projection | $\max(0,\ \mathrm{target}-\mathrm{power})$ | Power projection from hire / mil build |
| Research | Remaining research work on leaf (+ missing prereqs); in-flight uses `tech_days_left` / innovation remaining | Research progress drained on that track |
| Law | Remaining enactment work (`law_days_left` when in flight) | Enactment progress toward passage |
| Interest | Remaining establishment work (`interest_days_left` when in flight) | Interest progress toward declared |
| Hire-to-full | Staffing gap or remaining hire days | Staffing / hire progress |
| AND/OR/NOT | DAG over children | $h=\max$/$\min$ |

### Notes per kind

**Price band / power projection.** Same $G/R^{*}$ shape as GDP. Replace GDP
deltas with price-progress or power-projection deltas; $h$ stays in **days**.
GDP hat/tilde formulas do not apply; use the corresponding meter’s cheap
guesstimate if needed.

**Research / law / interest.** `evaluate` is still boolean (`has_tech` /
`has_law` / interest held). Ranking residual $G$ should be **remaining track
work**, not that boolean — partial progress shortens $h$ and $T_i$. Today
`simple_subgoal_gap` still returns $0$/$1$ for those leaves and ranking often
jumps straight to ETA (`research_eta_for_rank`, fixed `law_days` /
`interest_days`); the table above is the intended meter.

Consistency: do not drop $h$ to $0$ merely because an item is queued on a 0-day
decision edge.

**Hire.** Same idea as other timed tracks: remaining staffing / hire days as
$G$; progress on complete as $\Delta$.

**Composites (goal DAG).**

| Node | $h$ |
| --- | --- |
| AND | $\max$ over children |
| OR | $\min$ over children |
| NOT | $0$ |
| Simple | meter plug-in from the table |

$h_{\mathrm{rank}}$ combines children with $\max$/$\min$. Multi-leaf goals
still drive unsatisfied leaves.
