# Goal DSL

Parser: **chumsky** combinators in `vic3-goals`. Keep the module small; do not turn it into a compiler-front-end showcase. Richer error recovery is optional.

GOAP crates inspired the *shape* (goal = desired facts, actions = preconditions/effects). They are not the engine.

## Predicates

A goal is a boolean formula over a `PlanningState`.

```text
goal        := or
or          := and ("||" and)*
and         := unary ("&&" unary)*
unary       := "not" unary | atom
atom        := pred | "(" goal ")"

pred        := ident "(" args? ")"
            |  ident rel number
rel         := "<=" | ">=" | "<" | ">" | "=="
args        := arg ("," arg)*
arg         := ident "=" value
value       := ident | number | string
```

Examples:

```text
has_tech(nitroglycerin)
has_law(law_homesteading)
good_price(ammunition) <= 40
army_power_projection >= 100
weekly_balance >= 100
population_weighted_wealth >= 20
credit_headroom > 0
debt_principal <= 200
solvent
interest_in(state=alsace)
```

Sugar that compiles to predicates (not evaluated as opaque strings):

```text
declare-war(state=alsace)
research(tech=nitroglycerin)
gdp >= 50e6
```

Optional `tag=` / `wargoal=` on `declare-war` parse for forward compatibility but are
**ignored by compile** today — they do not become atoms. Prefer `state=` (or
`region=`) alone in the UI and new examples.

## Compilation: `declare-war`

Requires `state=` or `region=`. Always expands to (at least):

- **interest** in the target strategic region / state
- **army** power projection sufficient to start the play (model threshold, documented in rustdoc)
- **munitions-price** (and related mil goods) under a ceiling derived from defs / opts
- **solvent** — known remaining credit before exhaustion (`principal < credit`)
  under frozen wages/employment; missing principal/credit leaves this false
  rather than guessing from treasury sign

Property (I-declare-war): compilation always includes those four conjuncts. Extra conjuncts (infamy headroom, relations, claims) may be added later without removing these. Infamy is projected on `PlanningState` already; it is not yet a declare-war conjunct.

`declare-war` is closable end-to-end when interest and army still need work **and**
munitions-price plus solvent already hold (or munitions can move under existing
building-level / PM actions). Interest and army use fixed-time queue + wait
successors; solvent uses the compact payday model when credit is exhausted.

## Compilation: `research`

`has_tech(x)` so the sim can `QueueTech` then event-wait. Innovation-capacity /
throughput conjuncts are **not** compiled yet — they remain sim-only if added
later.

## Compilation: `gdp`

Predicate on modeled current GDP in `PlanningState`: gross solved building
output value (the sum of non-negative building revenue) in states owned by the
selected country. It is recomputed after modeled construction or PM switches.
For an increasing GDP target, the simulator considers at most the three building
types with the highest current solved output value per added level, plus capped
SwitchPm alternatives when defs expose them.

## Evaluation

`vic3-goals` evaluates a compiled predicate against `PlanningState`. It does not search. Gaps = unsatisfied atoms. Plans = A* until the predicate is true ([`planning.md`](planning.md)).

UI presets and SQL `plan`/`gaps` TVFs consume the **same** compiled atoms — presets are DSL strings, not a parallel vocabulary.

## What search can close today

| Goal family | Gaps | Timeline (A*) |
| --- | --- | --- |
| `research` / `has_tech` | yes | yes (`QueueTech` + wait) |
| `has_law` | yes | yes (`QueueLaw` + wait) |
| `gdp` / supported `good_price` | yes | yes (bounded building levels and/or SwitchPm + re-solve) |
| `interest_in` / `army_power_projection` | yes | yes (`QueueInterest` / `QueueArmyPower` + wait) |
| `declare-war` | yes | yes when munitions + solvent hold (interest/army); solvent via payday |
| `solvent` / `credit_headroom` / `debt_principal` | yes | yes (payday waits on frozen weekly balance) |
| `weekly_balance` | yes | yes (`AdjustTax` on the frozen balance sample) |
| SoL proxy (`population_weighted_wealth`) | yes | **not yet** (need wage model) |

## Default plan presets

The web UI maps default plans to ordinary DSL; presets do not bypass parsing or
evaluation:

| Preset | Goal | Timeline |
| --- | --- | --- |
| Prepare for war | `declare-war(state=alsace)` | closable when munitions + solvent hold (or payday) |
| Good-sized military | `army_power_projection >= 100` | closable when PP known |
| Economic growth | `gdp >= 100000000` | closable |
| Increase weekly income | `weekly_balance >= 100` | closable (tax steps) |
| Avoid default | `credit_headroom > 0` | closable (payday model) |
| Raise standard of living | `population_weighted_wealth >= 20` | gaps only |

The state and numeric targets are starting values, not fixed policy.
`weekly_balance` reads the most recent finite saved net-budget sample; tax
adjust steps shift that sample by a model constant and do not recompute wages.
`population_weighted_wealth` averages saved pop wealth by household
population in states owned by the country; it is a compact SoL proxy, not a
recomputed living-standard equilibrium. `credit_headroom` is `credit - principal`
when both are present; `solvent` is true only when that headroom is strictly
positive. Under the payday model, surplus balance pays down principal before
increasing treasury, then headroom/solvent refresh. `army_power_projection` reads
country cache or army-formation power from the save when present; missing
projection is unknown (SQL `gaps` → `unknown`, not a silent zero). `interest_in(state=…)` /
`interest_in(region=…)` match projected state vs strategic-region interest ids
separately (Clausewitz ids are normalized, e.g. `STATE_ALSACE` → `alsace`).
`has_law(…)` matches projected active laws (`law_` prefix insensitive).
Missing metrics remain unavailable and do not satisfy comparisons. A valid
preset can still be unreachable when `vic3-sim` has no action capable of
closing its open atoms.

## Golden examples (P7a tests)

| Input | Must include |
| --- | --- |
| `declare-war(tag=FRA, wargoal=conquer_state, state=alsace)` | interest, army, munitions-price, solvent (`tag`/`wargoal` ignored) |
| `research(tech=nitroglycerin)` | `has_tech(nitroglycerin)` |
| `good_price(ammunition) <= 40 && solvent` | those two atoms |
