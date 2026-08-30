# Planning Search Scaling

Notes on planner A* scaling: what is implemented, what is deferred, and what we
rejected.

The live planning contract remains [`planning.md`](planning.md). Search still
uses `SearchNode` / `shortest_path` from `rust-advanced-heaps`.

---

## Implemented: PEA* adapter (country-wide top‑K)

Production `plan` / `plan_with_economy` wrap [`Vic3Node`] in [`PeaNode`]
(`crates/vic3-planning/src/plan/pea.rs`):

- Build one **national** candidate bag from domain successors (all placements /
  actions for that current node), scored by cheap bag keys for generation and
  by open-set `edge + h(child)` after speculative apply for beam partition
  (**once per row**, then `select_nth` on cached keys).
- Choose top [`DEFAULT_PEA_BEAM`] (**16**) with `select_nth` on the open delta
  (sort only that prefix). Defer the rest in one `Expanding` cursor (`Rc<[Candidate]>`).
- **Apply child only on emit** — bag rows store `action` / `days` / cheap score /
  deps, not a live [`Vic3Node`].
- Cursor heuristic is the best deferred open delta (`edge + h` on speculative child).
- ShopCache stays unranked (score/apply substrate only).

**Beam policy (locked for v1):** fixed width 16 (not ties-only). Motivated by a
no-refit GDP / profit-per-level proxy on Prussia 1836 (~50 build-level
candidates. Top 8–16 held the high-value head). Not a proven optimum — tune
with real planner histograms later.

This **defers OPEN insertion** per node expand. It is **not** a shared slot
budget across the whole A* frontier (that would need heaps-crate changes).

**Open-set ordering invariant:** after each partial expand, A* must dequeue
every emitted Ready child before the 0-cost `Expanding` resume cursor from that
same expand. Beam partition, emitted children, and the cursor therefore all use
`edge + h(child)` after speculative apply (production `h` is the admissible timing
bound on [`Vic3Node`]). Cheap bag keys are diagnostic only — mixing them into the
cursor heuristic caused resume nodes to jump ahead of siblings.

---

## Deferred (not implemented)

### EPEA*

[Enhanced Partial Expansion A*](https://www.jair.org/index.php/jair/article/view/10882)
generates only the current \(f\)-tier via an Operator Selection Function, skipping
surplus generation. Needs cheap \(\Delta f\) prediction. Hard where \(h\) needs
deep apply / economy resolve.

### Soft / ε partial-order reduction

Vic3 actions are **near-commutative** (composable state patches), not strictly
independent. Soft POR / stubborn sets remain research. Do not claim optimal
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

## Debug expand tracing (`VIC3_PLAN_TRACE`)

Env-gated stderr diagnostics for A* / PEA* expansions. Implementation:
[`astar_trace.rs`](../crates/vic3-planning/src/plan/astar_trace.rs). Lines are
**not** part of `PlanResult` JSON.

| Variable | Effect |
| --- | --- |
| `VIC3_PLAN_TRACE=1` | Enable (any non-empty value except `0` / `false`) |
| `VIC3_PLAN_TRACE_EVERY=N` | Log expand `#1` and every *N*-th expand after that (default **100**) |

Example:

```bash
VIC3_PLAN_TRACE=1 VIC3_PLAN_TRACE_EVERY=1 \
  vic3-cli plan --save … --game … --goal "gdp >= 450000"
```

### Line kinds

| Prefix / kind | When | Meaning |
| --- | --- | --- |
| `[astar] trace on (every N)` | Plan start | Trace enabled; expand counter reset. |
| `[astar] plan start …` | Before search | Root **gdp**, admissible **h** (days), **sim_branch** (immediate simulator successors), **max_days**. |
| `[astar] greedy_u=…` | Before search | Greedy upper bound on day cost (`Some(0)` ⇒ already at goal; `None` ⇒ no cap). Caps `PathFinderBuilder::max_cost` when `Some(u)` with `u > 0`. |
| `[astar] #N +Ts pea-ready …` | PEA Ready expand | National candidate bag scored; beam of `DEFAULT_PEA_BEAM` (16) emitted. Fields: **fp**, **gdp**, **h**, **candidates**, **beam**, **fp_dups** / **fp_uniques**. |
| `[astar] #N +Ts pea-resume …` | PEA Expanding cursor | Resume deferred bag. **h** here is the cursor’s deferred open heuristic (`edge + h(child)`), not domain `heuristic()`. **emitted** / **remaining** track beam progress. |
| `[astar] #N +Ts vic3 …` | Direct `Vic3Node` expand | Used when searching domain nodes without the PEA wrapper (tests / helpers). **branch** and action-kind histogram of `sim_successors`. |
| `[astar] GOAL after N …` | Goal closed | Domain **fp**, **gdp**, **date** at the satisfying node. |
| `[astar] plan done …` | After search | **expands**, **day_cost**, **path_len**, plus cumulative `SearchTraceStats` (below). |

Common expand fields:

| Field | Meaning |
| --- | --- |
| `#N` | 1-based expand index (throttled by `EVERY`). |
| `+Ts` | Seconds since `reset()`. |
| `fp` | Domain fingerprint (hex). |
| `gdp` | Modeled GDP at that node. |
| `h` | Timing lower bound (days), or deferred open *h* on `pea-resume`. |
| `fp_dups` / `fp_uniques` | Running domain-fingerprint reconvergence counters for the shared search context. |

`plan done` summary (`SearchTraceStats`):

| Field | Meaning |
| --- | --- |
| `fp_dups` / `fp_uniques` / `fp_emitted` | Fingerprint reuse vs first-seen; `fp_emitted = dups + uniques`. |
| `pea_ready` / `pea_resume` | Counts of Ready vs Expanding expands. |
| `ranked_max` / `ranked_avg` / `ranked_sum` | Candidate-bag sizes at Ready expands. |
| `beam_emitted` / `beam_deferred` | Successors pushed into the beam vs times an Expanding cursor was re-queued. |

True A* open/closed set sizes are unavailable (`rust-advanced-heaps` keeps them private); the note on the summary line is intentional.

**Keep in sync:** if you change `[astar]` line formats or field names, update this section and the emit sites that link here.

---

## Suggested later ladder

1. Measure PEA* OPEN size / wall time vs full expand on real goals. Retune beam.
2. EPEA*-style OSF where \(\Delta f\) is cheap (e.g. research simple subgoals).
3. Optional satisficing preferences — still not dominance theorems.
