# Planning Search Scaling

Notes on planner A* scaling: what is implemented, what is deferred, and what we
rejected.

The live planning contract remains [`planning.md`](planning.md). Search still
uses `SearchNode` / `shortest_path` from `rust-advanced-heaps`.

---

## Implemented: PEA* adapter (fixed beam)

Production `plan` / `plan_with_economy` wrap [`Vic3Node`] in [`PeaNode`]
(`crates/vic3-planning/src/plan/pea.rs`):

- Rank domain successors by \(f - g = \mathrm{edge} + h(\mathrm{child})\).
- Emit a **fixed beam** of [`DEFAULT_PEA_BEAM`] (**16**) children.
- Re-insert the parent as an `Expanding` cursor (0-cost self-edge) whose
  heuristic equals the next child's \(f - g\), so the closed-set loop can defer
  the surplus without a heaps-crate change.
- Path reconstruction drops cursor nodes and keeps domain fingerprint changes.

**Beam policy (locked for v1):** fixed width 16 (not ties-only). Motivated by a
no-refit GDP / profit-per-level proxy on Prussia 1836 (~50 build-level
candidates; top 8–16 held the high-value head). Not a proven optimum — tune
with real planner histograms later.

This **defers OPEN insertion**; it still materializes the full successor `Vec`
once per domain expand.

---

## Deferred (not implemented)

### EPEA*

[Enhanced Partial Expansion A*](https://www.jair.org/index.php/jair/article/view/10882)
generates only the current \(f\)-tier via an Operator Selection Function, skipping
surplus generation. Needs cheap \(\Delta f\) prediction; hard where \(h\) needs
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

## Suggested later ladder

1. Measure PEA* OPEN size / wall time vs full expand on real goals; retune beam.
2. EPEA*-style OSF where \(\Delta f\) is cheap (e.g. research atoms).
3. Optional satisficing preferences — still not dominance theorems.
