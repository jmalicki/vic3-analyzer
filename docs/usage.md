# Usage

CLI and UI are driven by the **same nested option structs**, and both call through **`vic3-api`** (bytes or path loaders → JSON). Filesystem fields (`save`, `--game`, `--tokens`) exist on CLI wrappers and on `vic3-api` path helpers; wasm takes `Vec<u8>` only. Inner option structs must not mention `PathBuf`.

P5 generates JSON Schema from these structs (schemars) and checks it against [`json-schema.md`](json-schema.md). The React form is built from schema, not a hardcoded field list.

## Environment

| Variable | Meaning |
| --- | --- |
| `VIC3_SAVE` | Path to a real `.v3` (live tests, ignored by default) |
| `VIC3_TOKENS` | Token map for binary saves |
| `VIC3_GAME` | Victoria 3 install (defs). Fixtures used in CI |

## CLI (target)

```text
vic3-cli prices --save game.v3 --tokens tokens.txt --game /path/to/Victoria3 --json
vic3-cli what-if --save game.v3 --building arms_industry --extra-levels 5 --json
vic3-cli alerts --save game.v3 --game /path/to/Victoria3 --json
vic3-cli mutate --save game.v3 --game /path/to/Victoria3 --delta-json '{...WorldDelta...}' --json
vic3-cli optimize-pms --save game.v3 --game /path/to/Victoria3 --axis income --json
vic3-cli export-save --save game.v3 --delta-json '{...SavePatch...}' --out new.v3
vic3-cli gaps --save game.v3 --goal 'declare-war(tag=FRA, wargoal=conquer_state, state=alsace)' --json
vic3-cli plan --save game.v3 --goal research(tech=nitroglycerin) --label 'rush explosives' --json
vic3-cli archive list
vic3-cli archive show <id>
vic3-cli archive diff <id> <id>
```

`--json` prints the result object (plus `limitations`). Without it, a short text table and a one-line limitations warning.

`mutate` prints a preview [`PricesResult`](json-schema.md) and does not write a file. `export-save` writes `--out` only; it never overwrites `--save`. `optimize-pms` `--axis` is `income`, `productivity`, or `sol`.

Flatten groups (clap in `vic3-cli` only):

- `SolveOpts` — `residual_eps`, `max_iters`, `warm_rel`, …
- `WhatIfOpts` — `building`, `extra_levels`, …
- `WorldDelta` — `--delta-json` on `mutate` (no `PathBuf` on the inner type)
- `AlertsResult` — `vic3-cli alerts` / wasm `loaded_alerts`
- `OptimizeResult` — `vic3-cli optimize-pms` / wasm `loaded_optimize_pms`
- `SavePatch` — `--delta-json` on `export-save`
- `PlanOpts` — `goal`, `max_days`, `label`, …
- Paths stay on the outer `*Cli` struct

## UI

1. Drop a `.v3` (and tokens if binary). Parse in-browser. Load **solves prices immediately**; there is no separate Analyze prices action.
2. Use the task navigation for **Prices**, **States**, **Pops**, **Alerts**, **Military**, **Buildings**, What-if, Timeline, Goal gaps, or Archive.
3. What-if uses building types read from the save; timeline and gaps use a guided goal builder, with the DSL retained as an advanced option.
4. Every run is saved to IndexedDB. Browse past saves, name alternative plans, and compare them (see [`archive.md`](archive.md)). Dropped saves also keep origin / timeline / step history so a reload restores the campaign.

### Default plans

Timeline and Goal gaps share presets for war readiness, military size, economic
growth, avoiding default, maximizing revenue, and standard of living. Choosing
an available preset fills both its goal and archive label; its fields remain
editable in the goal builder.

The current DSL can evaluate war readiness, an army-power target, GDP, credit
headroom / solvency from saved principal and credit, the latest saved net
weekly-budget sample, and a population-weighted saved-pop-wealth SoL proxy.
Missing fiscal or population metrics remain unavailable rather than being
treated as zero. Research, modeled GDP growth, and supported goods-price goals
can produce timelines; economic goals use bounded building-level expansions,
fixed modeled construction time, and a price re-solve. Military, fiscal, and
saved-wealth goals remain readiness diagnostics until their transition models
or save IR exist.

## Defs

The CLI reads a game install. The hosted wasm demo ships a tiny fixture blob,
which intentionally includes only a few goods. Build a complete, local blob
from your own install:

```text
vic3-cli defs export --game "/path/to/Victoria 3/game" --out defs.postcard
```

Upload that blob in the web UI. It is derived from Paradox game data and must
not be committed or published.

## Limitations in the answer

The residual and frozen-world caveats remain part of CLI/wasm result JSON and
archived records. The web UI keeps normal results focused on goods and actions,
shows a warning only when the solve did not converge, and links to
[`prices.md`](prices.md) for method details.
