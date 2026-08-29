# Planning Search Scaling

Notes on planner A* scaling: what is implemented, what is deferred, and what we
rejected.

The live planning contract remains [`planning.md`](planning.md). Search still
uses `SearchNode` / `shortest_path` from `rust-advanced-heaps`. Progress-aware
$h$, PEA ranking, and the **greedy incumbent** ($U$):
[`planning-progress-heuristic.md`](planning-progress-heuristic.md).

---

## Implemented: PEA* adapter (country-wide top‑K)

Production `plan` / `plan_with_economy` wrap [`Vic3Node`] in [`PeaNode`]
(`crates/vic3-planning/src/plan/pea.rs`):

- Build one **national** candidate bag from domain successors (all placements /
  actions for that current node), scored by **cheap**
  $\mathrm{score}_{\mathrm{cheap}} = e + \text{follow-on guesstimate}$
  via [`bag_rank::cheap_rank_bag`](../crates/vic3-planning/src/plan/bag_rank.rs)
  (which calls [`cheap_bag_score`](../crates/vic3-planning/src/plan/progress_h.rs) —
  not the admissible timing $h_{\mathrm{adm}}$).
- Choose top [`DEFAULT_PEA_BEAM`] (**16**) with `select_nth` (sort only that
  prefix); defer the rest in one `Expanding` cursor (`Rc<[Candidate]>`).
- **Apply child only on emit** — bag rows store `action` / `days` / cheap score;
  [`bag_rank::emit_child`](../crates/vic3-planning/src/plan/bag_rank.rs) applies
  the action and sets `gdp_for_rates`; emit rescore uses
  [`bag_rank::emit_rank_key`](../crates/vic3-planning/src/plan/bag_rank.rs)
  ($\mathrm{score}_{\mathrm{emit}}$ via [`emit_bag_score`](../crates/vic3-planning/src/plan/progress_h.rs)).
- Cursor heuristic is the best deferred **cheap** bag score after `select_nth`.
- ShopCache stays unranked (score/apply substrate only).

**Beam policy (locked for v1):** fixed width 16 (not ties-only). Motivated by a
no-refit GDP / profit-per-level proxy on Prussia 1836 (~50 build-level
candidates; top 8–16 held the high-value head). Not a proven optimum — tune
with real planner histograms later.

This **defers OPEN insertion** per node expand. It is **not** a shared slot
budget across the whole A* frontier (that would need heaps-crate changes).

### Known: mixed $f$ on the open set (tolerated for v1)

Bag order is **not** a true PEA $f$-layer.

- **Ready** nodes report A* heuristic $h_{\mathrm{adm}}$ (timing DAG on
  [`Vic3Node`](../crates/vic3-planning/src/plan/vic3.rs)).
- **Expanding** reports best deferred cheap bag score.
- Path cost $g$ is always calendar days.

$h_{\mathrm{rank}}$ / cheap bag scores are only a ranking bias. Within one bag,
later deferred rows are $\ge$ the cursor bound under that score. Across the
open set, when a deferred child becomes Ready its key $g + h_{\mathrm{adm}}$
can be **lower** than the Expanding cursor’s earlier key (or lower than nodes
already expanded). That $f$ drop looks like a negative step: classic PEA
assumes nondecreasing true $f$ on emit; we do not have that here.

**v1 tolerate:** keep bag scoring on progress/cheap ranks and Expanding’s
reported heuristic as today. Correctness still leans on day-cost $g$,
$h_{\mathrm{adm}}$ on Ready nodes, and incumbent $U$ — not on bag order being
a true $f$-order. A later fix may score bags with $h_{\mathrm{rank}}$ but set
Expanding’s A* heuristic from $h_{\mathrm{adm}}$ (or
$\mathrm{edge} + h_{\mathrm{adm}}$) so resume cannot invent $f$ drops.

When a beam emit’s $\mathrm{score}_{\mathrm{emit}}$ is **greater** than the
best deferred $\mathrm{score}_{\mathrm{cheap}}$, `tracing::warn!`
(`vic3_planning::pea`) fires — decided by
[`bag_rank::emit_deferred_cheap_mismatch`](../crates/vic3-planning/src/plan/bag_rank.rs)
(testable without a tracing subscriber).

---

## Deferred (not implemented)

### EPEA*

[Enhanced Partial Expansion A*](https://www.jair.org/index.php/jair/article/view/10882)
generates only the current $f$-tier via an Operator Selection Function, skipping
surplus generation. Needs cheap $\Delta f$ prediction; hard where $h$ needs
deep apply / economy resolve.

### Soft / ε partial-order reduction

Vic3 actions are **near-commutative** (composable state patches), not strictly
independent. Soft POR / stubborn sets remain research; do not claim optimal
POR under near-commutativity alone.

---

## Rejected for now: “true” dominance pruning

Site/type dominance (e.g. prune worse `+1 rye` while a better site exists, or
Pareto on immediate ΔGDP and days) looks attractive but is **too iffy** for a
correctness claim:

- Means-to-an-end (construction sectors, tooling, input chains) beat myopic GDP.
- Same building type still differs by market, labor, infra, and order effects
  when both will be built.
- Sound rules need continuation bounds or narrow peer-group proofs we do not
  have.

**Do not** implement dominance prunes as correctness. Soft preferences /
helpful-operator ordering may return later as heuristics only.

---

## GDP Fractional Knapsack Heuristic

For GDP goals, we implement a **Fractional Knapsack** relaxation to provide a tight, mathematically provable lower bound \(h\) for A*. 

Instead of a full greedy simulation (which would overestimate time and break A* admissibility by providing an upper bound), the planner precomputes the theoretical \(\Delta\mathrm{GDP} / \mathrm{CP}\) efficiency of all available buildings using optimistic base prices. 

During node evaluation, it fractionally consumes this perfectly sorted, idealized knapsack until the GDP target is reached. By assuming zero economic friction (no input shortages, no price crashes from oversupply, and infinite construction scaling if needed), we guarantee this theoretical "perfect economy" path will *always* take less time than any real gameplay simulation. This guarantees A* admissibility while keeping the heuristic perfectly \(O(1)\) to evaluate in the hot path.

---

## Suggested later ladder

1. Measure PEA* OPEN size / wall time vs full expand on real goals; retune beam.
2. EPEA*-style OSF where $\Delta f$ is cheap (e.g. research simple subgoals).
3. Tighten progress $R^{*}$ (real build/PM $\Delta$ predictions) per
   [`planning-progress-heuristic.md`](planning-progress-heuristic.md).
4. Optional later: incremental greedy membership maintain (v1 full-rebuilds $U$).
5. Optional satisficing preferences — still not dominance theorems.
