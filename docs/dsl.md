# Goal Domain-Specific Language (DSL)

The Goal DSL allows players, developers, and AI agents to define strategic targets as boolean formulas over a [`PlanningState`](planning.md).

These expressions power the **Goal Gaps** evaluator (which checks unsatisfied readiness conditions) and the **Timeline Planner** (which finds an optimal sequence of actions to achieve the goal).

---

## Grammar

Goals are parsed into boolean abstract syntax trees (AND, OR, NOT) over atomic predicates:

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

### Atomic Predicates

| Predicate | Example | Description |
| --- | --- | --- |
| `has_tech(id)` | `has_tech(nitroglycerin)` | True when the country has researched the technology |
| `has_law(id)` | `has_law(law_homesteading)` | True when the specified law is active |
| `good_price(id) [rel] [num]` | `good_price(ammunition) <= 40` | Market price ceiling or floor |
| `army_power_projection [rel] [num]` | `army_power_projection >= 100` | Army power projection threshold |
| `navy_power_projection [rel] [num]` | `navy_power_projection >= 100` | Navy power projection threshold |
| `weekly_balance [rel] [num]` | `weekly_balance >= 100` | Net weekly budget balance |
| `credit_headroom [rel] [num]` | `credit_headroom > 0` | Remaining credit before default (`credit - principal`) |
| `debt_principal [rel] [num]` | `debt_principal <= 200` | Total outstanding debt |
| `solvent` | `solvent` | True when `credit_headroom > 0` |
| `interest_in(state=... / region=...)` | `interest_in(state=alsace)` | Declared interest in state or strategic region |
| `population_weighted_wealth [rel] [num]`| `population_weighted_wealth >= 20` | Pop wealth average (Standard of Living proxy) |

---

## Compound Goal Sugar

Higher-level strategic goals expand into concrete conjuncts (atomic prerequisites):

### 1. `declare-war(state=...)`
Expands to all prerequisites needed to safely launch a diplomatic play:
- **Strategic Interest:** `interest_in(state=...)` in the target state or region.
- **Army Strength:** `army_power_projection >= [threshold]` sufficient to contest the play.
- **Munitions Supply:** `good_price(ammunition) <= [ceiling]` to prevent combat goods shortages.
- **Fiscal Solvency:** `solvent` (`credit_headroom > 0`) to prevent immediate bankruptcy during war mobilization.

### 2. `colonize(region=...)`
Expands to institutional, technological, and military prerequisites for colonial expansion:
- `has_tech(colonization)` and `has_tech(quinine)`
- `has_law(law_colonial_resettlement) || has_law(law_colonial_exploitation)`
- `interest_in(region=...)`
- `army_power_projection >= [threshold]` and `navy_power_projection >= [threshold]`
- `solvent`

### 3. `research(tech=...)`
Expands to `has_tech(...)`, allowing the planner to queue the technology and wait for research completion.

### 4. `gdp >= [amount]`
Evaluates gross solved building output value across owned states. When planning, the simulator prioritizes expanding high-output buildings and upgrading productive production methods.

---

## Planner Capabilities

| Goal Family | Readiness Gaps Check | Timeline Planner | Primary Planner Actions |
| --- | :---: | :---: | --- |
| `research` / `has_tech` | ✅ | ✅ | `QueueTech` + research time waits |
| `has_law` | ✅ | ✅ | `QueueLaw` + checkpoint waits |
| `gdp` / `good_price` | ✅ | ✅ | Defs-based building candidates (first-of-type, state-scoped enqueue, no dominance prune) + Construction Sector meta lever + `SwitchPm` + price re-solves |
| `interest_in` | ✅ | ✅ | `QueueInterest` + establishment wait |
| `army_power_projection` | ✅ | ✅ | Staffed barracks construction + recruitment |
| `navy_power_projection` | ✅ | ✅ | Shipyards + naval administrations construction |
| `declare-war` | ✅ | ✅ | Closes interest, military size, munitions, and debt |
| `colonize` | ✅ | ✅ | Closes techs, laws, interest, and military thresholds |
| `solvent` / `credit_headroom` | ✅ | ✅ | Payday debt reduction under frozen weekly surplus |
| `weekly_balance` | ✅ | ✅ | `AdjustTax` steps |
| `population_weighted_wealth` | ✅ | Diagnostic only | Awaits wage/labor dynamic model |

---

## Canonical Goal Examples

```text
# Military preparation for conflict over Alsace
declare-war(state=alsace)

# Rushing explosives technology
research(tech=nitroglycerin)

# Securing munitions price stability while maintaining positive credit
good_price(ammunition) <= 40 && solvent

# Raising GDP above 50 million with balanced budget
gdp >= 50000000 && weekly_balance >= 0
```
