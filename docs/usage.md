# Usage

CLI and UI are driven by the **same nested option structs**. Filesystem fields (`save`, `--game`, `--tokens`) exist only on CLI wrappers. wasm takes `Vec<u8>`. Inner structs must not mention `PathBuf`.

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
vic3-cli gaps --save game.v3 --goal 'declare-war(tag=FRA, wargoal=conquer_state, state=alsace)' --json
vic3-cli plan --save game.v3 --goal research(tech=nitroglycerin) --label 'rush explosives' --json
vic3-cli archive list
vic3-cli archive show <id>
vic3-cli archive diff <id> <id>
```

`--json` prints the result object (plus `limitations`). Without it, a short text table and a one-line limitations warning.

Flatten groups (clap in `vic3-cli` only):

- `SolveOpts` — `residual_eps`, `max_iters`, …
- `WhatIfOpts` — `building`, `extra_levels`, …
- `PlanOpts` — `goal`, `max_days`, `label`, …
- Paths stay on the outer `*Cli` struct

## UI (target)

1. Drop a `.v3` (and tokens if binary). Parse in-browser.
2. Price table + limitations under the table.
3. What-if form from exported schema (nested structs = fieldsets).
4. Gaps / plan screens later.
5. Every run can be saved to IndexedDB. Browse past saves, name alternative plans, compare (see [`archive.md`](archive.md)).

## Defs

CLI reads the game install. wasm ships a **prebuilt defs blob** from a supported patch. Mismatch with the save’s version is reported, not silently ignored.

## Limitations in the answer

The residual and the frozen-world caveats are **part of the result**, not a footnote we omit. CLI `--json.limitations`, rustdoc on `solve`, and the React table all carry the same strings.
