# Usage Guide

This guide covers running **`vic3-analyzer`** via the Command Line Interface (CLI) and the Web UI.

Both interfaces share the same underlying analysis engine ([`vic3-api`](../crates/vic3-api)) and emit identical JSON result schemas ([`json-schema.md`](json-schema.md)).

---

## Environment Variables

| Variable | Meaning | Notes |
| --- | --- | --- |
| `VIC3_SAVE` | Default path to a `.v3` save file | Used by CLI commands when `--save` is omitted |
| `VIC3_GAME` | Path to your installed `Victoria 3/game` directory | Used to read game definitions |
| `VIC3_DEFS` | Path to a prebuilt `defs.postcard` blob | Alternative to reading live game directory |
| `VIC3_TOKENS` | Token mapping file for binary Ironman saves | Optional; text saves do not require tokens |

---

## Command Line Interface (CLI)

The CLI binary (`vic3-cli`) provides fast batch analysis, what-if evaluations, qualification bottleneck alerts, and goal planning.

### Common Commands

```bash
# Calculate market and state-level equilibrium prices
vic3-cli prices --save campaign.v3 --game "/path/to/Victoria 3/game"

# Run a what-if analysis: simulate adding 5 levels of arms industries
vic3-cli what-if --save campaign.v3 --building arms_industry --extra-levels 5

# Check state-by-state pop employment shortages and qualification alerts
vic3-cli alerts --save campaign.v3 --game "/path/to/Victoria 3/game"

# Evaluate readiness gaps for a strategic goal
vic3-cli gaps --save campaign.v3 --goal "declare-war(state=alsace)"

# Plan a time-optimal action sequence to reach a goal
vic3-cli plan --save campaign.v3 --goal "research(tech=nitroglycerin)" --label "rush explosives"

# Optimize production methods for a target axis (income, productivity, or sol)
vic3-cli optimize-pms --save campaign.v3 --axis income

# Export a modified plaintext save with applied what-if changes
vic3-cli export-save --save campaign.v3 --delta-json '{"production_methods": [...]}' --out modified.v3

# Manage local plan archives
vic3-cli archive list
vic3-cli archive show <record-id>
vic3-cli archive diff <record-id-1> <record-id-2>
```

Adding `--json` to any command emits full structured JSON matching the schemas in [`json-schema.md`](json-schema.md).

---

## Web Application Workflow

1. **Load Save:** Drag and drop a `.v3` save file into the browser. Text saves are parsed immediately; binary saves prompt for a token map if not previously provided.
2. **Instant Price Solve:** On load, the browser immediately calculates market equilibrium prices and state-level MAPI blends.
3. **Tab Navigation:**
   - **Prices:** Explore global goods prices, volume-weighted averages, and drill down into state-level buy/sell orders and building economics.
   - **States & Pops:** Inspect regional infrastructure, arable land, pop wealth, and literacy.
   - **Alerts:** View actionable employment shortages and qualification feeder advice.
   - **What-If:** Test building level expansions or PM switches with live price re-solves.
   - **Goal Gaps & Timeline:** Evaluate readiness or compute step-by-step action sequences using the guided goal builder.
   - **Archive:** View past runs, branch alternative timelines, and compare plans.

---

## Default Plan Presets

The Web UI and CLI share standard presets for common strategic goals (defined using the [Goal DSL](dsl.md)):

| Preset | Goal Expression | How It Is Evaluated |
| --- | --- | --- |
| **Prepare for War** | `declare-war(state=...)` | Checks strategic interest, army power projection, munitions price ceiling, and credit solvency. |
| **Colonize Region** | `colonize(region=...)` | Checks colonization/quinine tech, colonial laws, naval/army power, and local interest. |
| **Economic Growth** | `gdp >= ...` | Solves GDP impact from expanding highest-output buildings or upgrading production methods. |
| **Military Expansion** | `army_power_projection >= ...` | Plans staffed barracks expansions and troop recruitment. |
| **Avoid Default** | `credit_headroom > 0` | Evaluates fiscal runway and plans debt reduction via the compact payday model. |
| **Raise Standard of Living** | `population_weighted_wealth >= ...` | Evaluates population-weighted wealth across domestic states (readiness gaps diagnostic). |

---

## Game Definitions Workflow

To analyze saves accurately, `vic3-analyzer` needs access to game definitions (goods base prices, building recipes, and pop need packages).

- **In the Desktop Companion & CLI:** Provide `--game "/path/to/Victoria 3/game"`. The tool reads definitions directly from your local installation.
- **In the Web App:** Drag and drop your local `Victoria 3/game` folder into the definitions modal. The browser extracts only the necessary definitions and goods icons, packaging them into a local `defs.postcard` blob stored in your browser's IndexedDB.
- **CLI Export:** You can pre-compile a definitions blob for fast CLI or web reuse:
  ```bash
  cargo run -p vic3-cli -- defs export --game "/path/to/Victoria 3/game" --out defs.postcard
  ```
  *(Note: Do not commit or redistribute `defs.postcard`, as it is derived from Paradox game data).*

---

## Understanding Result Caveats

Every price solve and planning run includes diagnostic solver information:
- **Residual:** Measures how closely the pop-consumption and market-price fixed point converged. A small residual ($\approx 10^{-6}$) indicates a consistent equilibrium.
- **Frozen Dimensions:** As documented in [Price Methodology](prices.md), base employment, wages, and trade-center volumes are held fixed from the save during pop re-equilibration unless explicitly modified by what-if actions.
