# Progress-aware planner heuristic

Developer internals for planner search. Related:
[`planning.md`](planning.md), [`planning-search.md`](planning-search.md).

**Status:** ranking / cheap-bag scorers live in `plan/progress_h.rs` and are
**wired into search** via `plan/bag_rank.rs` (`cheap_bag_score` /
`emit_bag_score` + `gdp_for_rates`). Greedy incumbent $U$ lives in
`plan/greedy.rs` and is wired into `plan()` via `PathFinderBuilder::max_cost`.

Code: `plan/progress_h.rs`, `plan/bag_rank.rs`, `plan/pea.rs`, `plan/greedy.rs`, `plan/result.rs`.

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

**Timeline states:** $\mathrm{state}_t$ is the node being expanded (current).
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
| $h$, $h_{\mathrm{rank}}$ | Progress residual-days estimate. **ranking bias**, not a proven admissible bound | `rank_heuristic` / `rank_heuristic_with_gdp_for_rates` |
| $U$ | Incumbent feasible plan length in days | `greedy_upper_bound` |
| $t_{\mathrm{goal}}^{\mathrm{greedy}}$ | Calendar day when greedy first satisfies the goal. $U$ equals this from the root | — |

1. **Cost is calendar days.** Find a shortest path in days until GDP reaches
   the target (`evaluate` true).
2. **A\*** / open-set search use $f = g + h$ with $g$ = days so far and edge costs in days
   (0-day decisions, positive waits). Ready keys still report $h_{\mathrm{adm}}$. Candidate bags / Expanding cursor use $h_{\mathrm{rank}}$ / cheap scores (mixed $f$ —
   see [`planning-search.md`](planning-search.md)).
3. **Upper bound $U$ comes from greedy.** Run a feasible greedy simulation to
   the GDP goal. $U$ is **only** that run’s day length (decisions are not kept).
   Prune any node with $g \ge U$ (`PathFinderBuilder::max_cost` in `plan()`).
   **v1:** when $U$ must be refreshed, **rebuild the whole greedy simulation**
   (no retained plan to edit. No incremental membership).
4. **$h_{\mathrm{rank}}$ only orders.** Progress residual scores rank the
   candidate bag / open bias. They are **not** a proven admissible lower bound and do
   **not** define $U$.

$$
U = t_{\mathrm{goal}}^{\mathrm{greedy}}
$$

The same day-cost / $U$ / prune skeleton applies to other goals. Only the
progress meter changes — see [Other goal types](#other-goal-types-how-the-gdp-story-changes).

---

## Incumbent upper bound $U$ (greedy defines days)

$U$ is the **length in calendar days** of one concrete greedy simulation that
satisfies the goal (`evaluate` true). It is **not** derived from $h_{\mathrm{rank}}$.

**v1 contract:** `greedy_upper_bound` returns a **scalar day count** for prune
only. It does **not** return or retain the decision sequence. Search does not
edit, simplify, or splice greedy picks — the loop invents decisions while
simulating, then **discards** them once $U$ is known. A later “incremental
membership” pass may keep an editable intended sequence. That is **not** this
PR / not v1.

- From the **root**, greedy runs to completion → $U$ is total plan days.
- From a **search node** (optional refine), greedy builds only the
  **remaining** suffix → incumbent path cost is $g + U_{\mathrm{remaining}}$.

### Seeding and refreshing $U$

Before or interleaved with planning search, run a **full** greedy from the root to
seed $U$. **v1:** any later refresh **re-runs the whole greedy simulation** from
the chosen start state (no patching a stored decision list. No incremental
membership).

- A feasible plan of length $U$ proves $\mathrm{optimal} \le U$.
- Greedy **never** enqueues a **new** Construction Sector. Excluding CS picks
  from greedy is **costless** (shortage-greedy would not choose CS — it does
  not produce the scarce good) and **protects** future incremental greedy
  assumptions (when/if we keep a membership set — not required for scalar $U$).
- When rebuilding greedy from a world that **already has CS enqueued** by
  search, greedy still does not pick a *new* CS. It waits the real
  construction timeline (slots / points) until that job completes.
- Ordinary productive builds are allowed. Capacity always has at least one
  government feed slot floor
  (base capacity floor). So GDP greedy can get a finite $U$ by enqueueing a
  logging camp (etc.) even when Construction Sector is never chosen.

### Greedy loop (simulates days. V1 discards the path)

The steps below describe how the day count is produced. **v1 does not persist**
this timeline as an editable plan — only $U$ is returned to search.

1. If `evaluate(goal)` already holds → $U = 0$.
2. While a track has a spare slot, enqueue the greedy pick among allowed
   0-day decisions (edge cost $0$). Allowed picks include ordinary
   `QueueBuildingLevel` jobs. **exclude** Construction Sector.
3. **Advance time** to the next completion (or payday), apply, and count the
   landing day (v1 need not store a timeline row — only the running day total).
4. When the GDP goal is satisfied, stop. **$U = t$** (current day).
   Do not enqueue further “just in case” work.
5. If nothing helps and no wait advances the goal → stop with no greedy $U$.

Search may still find a cheaper path (including Construction Sector) and
tighten $U$.

### Future: incremental membership (not v1)

Deferred. A later pass may keep an intended sequence + membership hash,
consume in-set edges, end-subtract when off-plan still reaches the goal, etc.
Until then: **always full-rebuild** greedy when refreshing $U$.

Hard rule that survives that future: greedy must **never** enqueue Construction
Sector.

### What to pick (GDP / build-fed)

Among legal `QueueBuildingLevel` successors **except Construction Sector**,
pick a building that **produces the highest-shortage good** we are allowed to
build toward.

Shortage signal (domestic market): compare each good’s current market price
to its defs **base** price — e.g. $p / p_{\mathrm{base}}$ (or the same ratio
as a rank key). Higher means more scarce / over base. Then:

1. Prefer the good with the highest signal among goods some **allowed**
   candidate building can produce.
2. Enqueue one allowed building that outputs that good (placement /
   per-type caps already filter “allowed”).

That is the greedy build policy — not rate-on-gap scoring, and not an
optional tie-break.

**PM / tax / other instant decisions:** if a 0-day option helps GDP (or clears
the gap), take it before waiting. Otherwise fall through to builds / waits.

### Pruning with $U$ (safe)

- Drop / never expand a node with $g \ge U$.
- Optional: $g + h_{\mathrm{adm}} \ge U$ when $h_{\mathrm{adm}}$ is a true lower bound.
- Do **not** prune on $g + h_{\mathrm{rank}} \ge U$ alone.

### Role split

| Quantity | Role |
| --- | --- |
| $h_{\mathrm{rank}}$ | Order open-set candidates (wired via `bag_rank`) |
| $U = t_{\mathrm{goal}}^{\mathrm{greedy}}$ | Feasible plan days. Prune with $g$ |
| Partial-expansion cursor | Eventually emits deferred candidates |

### Improving $U$ later

When anything produces a **better feasible** day cost $U' < U$ (search goal
hit, or a **full** greedy rebuild from a better state):

1. Set $U \leftarrow U'$.
2. **Keep** open / closed / partial-expansion state.
3. Tighten prune: ignore nodes with $g \ge U$.
4. Do not rebuild ranking bags solely because $U$ changed.

**v1:** do not patch the old greedy timeline in place — run the greedy loop
again if a new incumbent path is needed.

### Construction Sector vs ordinary builds

“Skip construction on greedy” means **skip Construction Sector only**, not
skip every `QueueBuildingLevel`.

Why exclude CS: (1) shortage-greedy would not pick it anyway. (2) a CS enqueue
changes construction capacity for everything after it, which would make
incremental greedy refresh unreliable. Search may still enqueue CS. Greedy must not *pick* new CS (but honors search-enqueued CS on rebuild).

[`ConstructionSlots::ONE`](../crates/vic3-planning/src/construction.rs) is the
parallel-feed floor so a productive building can always be considered when
capacity admits a job.

### Bookkeeping the incumbent

**v1 (shipped):** search prune needs **only** the scalar $U$.
`greedy_upper_bound` does not return actions, a timeline, or a membership set —
there is nothing to edit or simplify after the run.

**Optional later (debug / incremental):** keep a greedy timeline or intended
sequence for explanation or membership refresh:

| Math | Definition | Rust / notes |
| --- | --- | --- |
| $t$ | Calendar day when a timeline event lands | future timeline row (not v1) |
| $\Delta$ | Predicted GDP change at that event | same units as the GDP gap |
| $G_{\mathrm{after}}$ | Residual GDP gap after $\Delta$ | — |

No membership set until the future incremental pass. If search reaches a world
it already solved, copy that world’s prices/GDP instead of running the price
solver again.

---

## Progress ranking for GDP

Used to order search candidate bags (wired via `bag_rank`). New symbols
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
- Production methods (`SwitchPm` when enabled. Prefer immediate $\Delta$ credit)
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
- $S_{\mathrm{build}} = 0$ ⇒ do not divide by zero. Advance time on the
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

## PEA ranking (cheap bag vs emit) — GDP

Bag order is a **bias**, not a true PEA $f$-layer — see
[`planning-search.md`](planning-search.md#known-mixed-f-on-the-open-set-tolerated-for-v1).
Do **not** prune on $g + h_{\mathrm{rank}} \ge U$.

New symbols for cheap vs emit scoring (quantities on $\mathrm{state}_t$ use
subscript $t$ / Rust `*_curr`. Successor worlds use $t'$ / `*_succ`):

| Math | Definition | Rust |
| --- | --- | --- |
| $\widetilde{\Delta\mathrm{gdp}}(a)$ | **Cheap** GDP change guesstimate for $a$ (not a full price solve. Not a bag key by itself) | `cheap_gdp_delta_guesstimate` |
| $\widehat{\Delta\mathrm{gdp}}(a)$ | **Emit-time** GDP change from a full price/GDP solve on the speculative completed successor | `speculative_completed_state` → `gdp_succ - gdp_curr` |
| $\mathrm{gdp}_t$ | GDP used for rates on $\mathrm{state}_t$ | `gdp_curr` / domain `gdp_for_rates()` |
| $\mathrm{gdp}_{\mathrm{for\_rates}}$ | GDP input to residual math on a node. After emit, $\mathrm{gdp}_t + \widehat{\Delta\mathrm{gdp}}$ so the successor anticipates completion before `state.gdp` updates | `Vic3Node::gdp_for_rates` |
| $G_t$, $R^{*}_t$, $H_{\mathrm{follow}}$ | Gap, aggregate rate, and follow-on days on $\mathrm{state}_t$ (one bag) | [`BagResidualCurr`](../crates/vic3-planning/src/plan/progress_h.rs) |
| $T_b$, $T_{\mathrm{CS}}$ | Days until ordinary build / Construction Sector completion | construction ETA helpers |
| $C$ | Construction points/day on $\mathrm{state}_t$ | `PlanningState::construction_points_per_day` |
| $\Delta C$ | Points/day from +1 Construction Sector (× government share) | `cheap_construction_sector_points_delta` |
| — | $\mathrm{state}_t$ context for every cheap score in one expand | [`CheapBagCurr`](../crates/vic3-planning/src/plan/progress_h.rs) |
| $\mathrm{score}_{\mathrm{cheap}}(a)$ | **Bag** order key (cheap follow-on) | `cheap_bag_score` |
| $\mathrm{score}_{\mathrm{emit}}(a)$ | Emit key $e(a) + h(W_a)$ | `emit_bag_score` |
| $W_a$ | Speculative successor after $a$’s economics land ($\mathrm{state}_{t'}$ with completion applied) | `speculative_completed_state` |

**Tilde vs hat:** $\widetilde{\cdot}$ = cheap bag approximation. $\widehat{\cdot}$ = emit full solve. Neither redefines path cost $g$.

Always $\mathrm{score}(a) = e(a) + \text{(follow-on days after } a\text{)}$.
**Cheap bag** and **emit** use different follow-on estimates.

### Cheap bag (order only)

Ordinary productive build (capacity unchanged. Credit
$\widetilde{\Delta\mathrm{gdp}}$):

$$
\mathrm{score}_{\mathrm{cheap}}(b)
  = T_b + \frac{\max\bigl(0,\ G_t - \widetilde{\Delta\mathrm{gdp}}(b)\bigr)}{R^{*}_t}
$$

Construction Sector (guesstimate — construction-unit scale of follow-on on
$\mathrm{state}_t$. Ignores CS $\widetilde{\Delta\mathrm{gdp}}$ /
$\widehat{\Delta\mathrm{gdp}}$):

$$
\mathrm{score}_{\mathrm{cheap}}(\mathrm{CS})
  = T_{\mathrm{CS}} + \frac{C}{C + \Delta C}\, H_{\mathrm{follow}}
$$

**Deficiencies vs emit / formal greedy** (also in code comments on
`cheap_bag_score`): CS scale ≠ actual slots + CS finish day. Cheap GDP
guesstimate ≠ full price solve. Bag does not re-run greedy. The delayed /
follow-on portion can score **lower (better)** than this heuristic once slots,
prices, and greedy order are real.

### Emit (expensive)

$$
\mathrm{score}_{\mathrm{emit}}(a) = e(a) + h(W_a)
$$

`emit_bag_score` takes a completed `PlanningState` and returns
$e(a) +$ [`rank_heuristic_with_gdp_for_rates`](../crates/vic3-planning/src/plan/progress_h.rs)
on that state’s GDP. Full residual $h(W_a)$ uses real post-CS capacity when relevant.
[`bag_rank::emit_child`](../crates/vic3-planning/src/plan/bag_rank.rs) caches
$\widehat{\Delta\mathrm{gdp}}$ on the emitted successor as `gdp_for_rates`.

$$
\mathrm{gdp}_{\mathrm{for\_rates}}
  = \mathrm{gdp}_t + \widehat{\Delta\mathrm{gdp}}(a).
$$

If $\mathrm{score}_{\mathrm{emit}}(a)$ is greater than the best deferred
$\mathrm{score}_{\mathrm{cheap}}$, `tracing::warn!` (`vic3_planning::pea`) —
[`bag_rank::emit_deferred_cheap_mismatch`](../crates/vic3-planning/src/plan/bag_rank.rs).

Tie-break: higher $r_i$ for that candidate, then fingerprint.

---

## When search considers a new successor

Expensive work is the price/GDP solve and progress scoring. Duplicate worlds
show up often (two different action sequences, same queues and buildings). Do
**not** pay for a solve only to discover A* already has that world.

Target order when expanding $\mathrm{state}_t$:

1. **Apply the action to planning fields only** — copy-on-write the queues,
   PMs, building levels, dates, etc. Do **not** run the price solver yet.
   Prices and GDP are derived. They are not part of A* identity.
2. **Look up that world in the open/closed map** keyed by fingerprint (and
   full state equality on collision). Remember the best path cost $g$ seen for
   that world so far.
3. **If this path is no better** ($g_{\mathrm{new}} \ge g_{\mathrm{best}}$): drop
   the successor. Skip the price solve, skip progress ranking, skip greedy.
4. **If this is a new world, or a cheaper path to a known world:**
   - New world: run the price/GDP solve once, then compute progress scores for
     PEA / ranking, and store the solved prices on the map entry.
   - Cheaper path to a known world: copy the **already solved** prices/GDP from
     the map entry onto this node. Do not solve again. Update $g_{\mathrm{best}}$.
5. **Greedy / $U$ refresh** is not per successor. Only when you promote a new
   feasible plan or explicitly rebuild the incumbent (full greedy rebuild in
   v1).

Also safe before any solve: if path cost $g$ is already $\ge U$, prune.

**Why:** identity ignores `good_prices` / `gdp`. Solving first, then detecting
a duplicate, wastes the solve. Today’s code often refreshes prices inside
`apply` and ranks before the pathfinder merges — this section is the intended
order, not what ships yet.

---

## Worked example (GDP)

$G = 1000$ (GDP shortfall), $r_A = 10$, $r_B = 20$, $S_{\mathrm{build}} = 2$.

- One imagined slot at B-rate: $h \approx 1000 / 20 = 50$
- One imagined slot at A-rate: $h \approx 1000 / 10 = 100$
- Two spare slots at B-rate: $R^{*}_{\mathrm{build}} = 20 + 20 = 40$,
  $h \approx 1000 / 40 = 25$
- Queue A and B together: $+200$ lands at day 10 while A continues. A free
  slot can take another B. After the second enqueue, predicted in-flight must
  include A so $h$ beats “A alone with idle spare capacity.”

---

## Other goal types (how the GDP story changes)

Day cost $g$, greedy $U$, prune on $g \ge U$, and “$h_{\mathrm{rank}}$ is bias
only” are **unchanged**. Swap the
progress meter ($G$, $\Delta$, which options feed $R^{*}$) and, where
relevant, GDP-specific cheap formulas.

| Simple subgoal kind | $G$ | $\Delta_i$ (examples) |
| --- | --- | --- |
| **GDP** (body of this doc) | $\max(0,\ \mathrm{target}-\mathrm{gdp})$ | Predicted GDP from build / PM / … |
| `good_price` band | Distance outside $[\mathrm{lo},\mathrm{hi}]$ | Move toward band (or $1$ if enters) |
| Army / navy power projection | $\max(0,\ \mathrm{target}-\mathrm{power})$ | Power projection from hire / mil build |
| Research | Remaining research work on leaf (+ missing prereqs). In-flight uses `tech_days_left` / innovation remaining | Research progress drained on that track |
| Law | Remaining enactment work (`law_days_left` when in flight) | Enactment progress toward passage |
| Interest | Remaining establishment work (`interest_days_left` when in flight) | Interest progress toward declared |
| Hire-to-full | Staffing gap or remaining hire days | Staffing / hire progress |
| AND/OR/NOT | DAG over children | $h=\max$/$\min$ |

### Notes per kind

**Price band / power projection.** Same $G/R^{*}$ shape as GDP. Replace GDP
deltas with price-progress or power-projection deltas. $h$ stays in **days**.
GDP hat/tilde formulas do not apply. Use the corresponding meter’s cheap
guesstimate if needed.

**Research / law / interest.** `evaluate` is still boolean (`has_tech` /
`has_law` / interest held). Ranking residual $G$ should be **remaining track
work**, not that boolean — partial progress shortens $h$ and $T_i$. Today
`simple_subgoal_gap` still returns $0$/$1$ for those leaves and ranking often
jumps straight to ETA (`research_eta_for_rank`, fixed `law_days` /
`interest_days`). The table above is the intended meter.

Consistency: do not drop $h$ to $0$ merely because an item is queued on a 0-day
decision edge.

**Hire.** Same idea as other timed tracks: remaining staffing / hire days as
$G$. Progress on complete as $\Delta$.

**Composites (goal DAG).**

| Node | $h$ |
| --- | --- |
| AND | $\max$ over children |
| OR | $\min$ over children |
| NOT | $0$ |
| Simple | meter plug-in from the table |

$h_{\mathrm{rank}}$ combines children with $\max$/$\min$. Multi-leaf goals
still drive unsatisfied leaves.
