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
declare-war(tag=FRA, wargoal=conquer_state, state=alsace)
research(tech=nitroglycerin)
gdp >= 50e6
```

## Compilation: `declare-war`

Always expands to (at least):

- **interest** in the target strategic region / state
- **army** power projection sufficient to start the play (model threshold, documented in rustdoc)
- **munitions-price** (and related mil goods) under a ceiling derived from defs / opts
- **solvent** — known remaining credit before exhaustion (`principal < credit`)
  under frozen wages/employment; missing principal/credit leaves this false
  rather than guessing from treasury sign

Property (I-declare-war): compilation always includes those four conjuncts. Extra conjuncts (infamy headroom, relations, claims) may be added later without removing these.

## Compilation: `research`

`has_tech(x)` plus queued-tech / innovation capacity constraints so the sim can `QueueTech` then event-wait.

## Compilation: `gdp`

Predicate on the country’s GDP series / current GDP in `PlanningState`, after prices.

## Evaluation

`vic3-goals` evaluates a compiled predicate against `PlanningState`. It does not search. Gaps = unsatisfied atoms. Plans = A* until the predicate is true ([`planning.md`](planning.md)).

## Default plan presets

The web UI maps default plans to ordinary DSL; presets do not bypass parsing or
evaluation:

| Preset | Goal |
| --- | --- |
| Prepare for war | `declare-war(tag=FRA, wargoal=conquer_state, state=alsace)` |
| Good-sized military | `army_power_projection >= 100` |
| Economic growth | `gdp >= 100000000` |
| Increase weekly income | `weekly_balance >= 100` |
| Avoid default | `credit_headroom > 0` |
| Raise standard of living | `population_weighted_wealth >= 20` |

The country, state, and numeric targets are starting values, not fixed policy.
`weekly_balance` reads the most recent finite saved net-budget sample.
`population_weighted_wealth` averages saved pop wealth by household population
in states owned by the country; it is a compact SoL proxy, not a recomputed
living-standard equilibrium. `credit_headroom` is `credit - principal` when both
are present; `solvent` is true only when that headroom is strictly positive.
Missing metrics remain unavailable and do not satisfy comparisons. A valid
preset can still be unreachable when `vic3-sim` has no action capable of
closing its open atoms.

## Golden examples (P7a tests)

| Input | Must include |
| --- | --- |
| `declare-war(tag=FRA, wargoal=conquer_state, state=alsace)` | interest, army, munitions-price, solvent |
| `research(tech=nitroglycerin)` | `has_tech(nitroglycerin)` |
| `good_price(ammunition) <= 40 && solvent` | those two atoms |
