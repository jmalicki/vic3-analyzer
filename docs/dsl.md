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
- **solvent** — can pay the army without an immediate default under frozen wages/employment

Property (I-declare-war): compilation always includes those four conjuncts. Extra conjuncts (infamy headroom, relations, claims) may be added later without removing these.

## Compilation: `research`

`has_tech(x)` plus queued-tech / innovation capacity constraints so the sim can `QueueTech` then event-wait.

## Compilation: `gdp`

Predicate on the country’s GDP series / current GDP in `PlanningState`, after prices.

## Evaluation

`vic3-goals` evaluates a compiled predicate against `PlanningState`. It does not search. Gaps = unsatisfied atoms. Plans = A* until the predicate is true ([`planning.md`](planning.md)).

## Golden examples (P7a tests)

| Input | Must include |
| --- | --- |
| `declare-war(tag=FRA, wargoal=conquer_state, state=alsace)` | interest, army, munitions-price, solvent |
| `research(tech=nitroglycerin)` | `has_tech(nitroglycerin)` |
| `good_price(ammunition) <= 40 && solvent` | those two atoms |
